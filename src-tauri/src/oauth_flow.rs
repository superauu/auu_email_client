//! One-use loopback callbacks; no authorization code or token is exposed through IPC.
use super::*;
use crate::{account_service, mail_store::RealAccount};
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri_plugin_opener::OpenerExt;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub id: String,
    pub state: String,
    pub account_id: Option<String>,
    pub error: Option<String>,
    pub folder_count: Option<usize>,
}
struct Entry {
    snapshot: Snapshot,
    cancel: Arc<AtomicBool>,
    created: Instant,
}
static FLOWS: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
fn flows() -> &'static Mutex<HashMap<String, Entry>> {
    FLOWS.get_or_init(Default::default)
}
fn unavailable() -> AppError {
    AppError::storage("授权会话不可用")
}
fn stopped(cancel: &AtomicBool) -> Result<(), AppError> {
    if cancel.load(Ordering::Acquire) {
        Err(AppError {
            code: "cancelled",
            message: "授权已取消，账号未保存".into(),
            retryable: false,
        })
    } else {
        Ok(())
    }
}
fn phase(
    id: &str,
    state: &str,
    error: Option<String>,
    account_id: Option<String>,
    count: Option<usize>,
) -> Result<(), AppError> {
    let mut flows = flows().lock().map_err(|_| unavailable())?;
    let flow = flows.get_mut(id).ok_or_else(unavailable)?;
    stopped(&flow.cancel)?;
    flow.snapshot = Snapshot {
        id: id.into(),
        state: state.into(),
        account_id,
        error,
        folder_count: count,
    };
    Ok(())
}
pub(crate) fn status(id: &str) -> Result<Snapshot, AppError> {
    flows()
        .lock()
        .map_err(|_| unavailable())?
        .get(id)
        .map(|f| f.snapshot.clone())
        .ok_or_else(|| AppError::invalid("授权会话已失效，请重新开始"))
}
pub(crate) fn cancel(id: &str) -> Result<Snapshot, AppError> {
    let mut flows = flows().lock().map_err(|_| unavailable())?;
    let flow = flows.get_mut(id).ok_or_else(unavailable)?;
    if flow.snapshot.state == "committing" {
        return Err(AppError::conflict("账号正在完成保存，请等待结果"));
    }
    if !["saved", "failed", "cancelled"].contains(&flow.snapshot.state.as_str()) {
        flow.cancel.store(true, Ordering::Release);
        flow.snapshot.state = "cancelled".into();
        flow.snapshot.error = None;
    }
    Ok(flow.snapshot.clone())
}
fn authorization_url(
    config: &Config,
    redirect: &str,
    state: &str,
    verifier: &str,
    user: &str,
) -> Result<String, AppError> {
    config.validate()?;
    let mut url = url::Url::parse(&config.authorization_endpoint())
        .map_err(|_| AppError::invalid("授权地址无效"))?;
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect)
        .append_pair("scope", config.scope())
        .append_pair("state", state)
        .append_pair("code_challenge", &pkce(verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("login_hint", user);
    if config.provider == Provider::Google {
        url.query_pairs_mut()
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
    } else {
        url.query_pairs_mut()
            .append_pair("response_mode", "query")
            .append_pair("prompt", "select_account");
    }
    Ok(url.into())
}
fn callback_target(
    request: &str,
    authority: &str,
    state: &str,
) -> Result<Option<String>, AppError> {
    let mut lines = request.split("\r\n");
    let mut first = lines.next().unwrap_or("").split_ascii_whitespace();
    if first.next() != Some("GET") {
        return Ok(None);
    }
    let Some(target) = first.next() else {
        return Ok(None);
    };
    if first.next() != Some("HTTP/1.1")
        || first.next().is_some()
        || !target.starts_with('/')
        || target.len() > 8192
    {
        return Ok(None);
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts.len() != 1 || !hosts[0].eq_ignore_ascii_case(authority) {
        return Ok(None);
    }
    let url = url::Url::parse(&format!("http://{authority}{target}"))
        .map_err(|_| AppError::auth("授权回调格式错误"))?;
    if url.path() != "/callback" {
        return Ok(None);
    }
    let mut params = HashMap::new();
    for (key, value) in url.query_pairs() {
        if ["state", "code", "error"].contains(&key.as_ref())
            && params.insert(key.to_string(), value.to_string()).is_some()
        {
            return Ok(None);
        }
    }
    let Some(actual) = params.get("state") else {
        return Ok(None);
    };
    let equal = actual.len() == state.len()
        && actual
            .bytes()
            .zip(state.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0;
    if !equal {
        return Ok(None);
    }
    if params.contains_key("error") {
        return Err(AppError::auth("授权未获批准，账号未保存"));
    }
    Ok(params.remove("code").filter(|code| {
        !code.is_empty() && code.len() <= 8192 && !code.chars().any(char::is_control)
    }))
}
fn respond(socket: &mut TcpStream, ok: bool) {
    let body = if ok {
        "Authorization received. Return to AuuMail."
    } else {
        "Invalid authorization callback."
    };
    let reply=format!("HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",if ok{"200 OK"}else{"400 Bad Request"},body.len(),body);
    let _ = socket.write_all(reply.as_bytes());
}
fn wait(
    listener: TcpListener,
    authority: &str,
    state: &str,
    cancel: &AtomicBool,
) -> Result<String, AppError> {
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut attempts = 0;
    loop {
        stopped(cancel)?;
        if Instant::now() >= deadline {
            return Err(AppError::auth("浏览器授权已超时，请重新开始"));
        }
        match listener.accept() {
            Ok((mut socket, peer)) => {
                attempts += 1;
                if !peer.ip().is_loopback() || attempts > 128 {
                    return Err(AppError::auth("授权回调过多，已停止会话"));
                }
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .and_then(|_| socket.set_write_timeout(Some(Duration::from_secs(1))))
                    .map_err(|_| AppError::network("无法设置授权回调超时"))?;
                let mut raw = vec![];
                let request_deadline = Instant::now() + Duration::from_secs(2);
                while raw.len() < 16384
                    && !raw.ends_with(b"\r\n\r\n")
                    && Instant::now() < request_deadline
                {
                    stopped(cancel)?;
                    let mut byte = [0];
                    match socket.read(&mut byte) {
                        Ok(1) => raw.push(byte[0]),
                        _ => break,
                    }
                }
                let result = if raw.ends_with(b"\r\n\r\n") {
                    std::str::from_utf8(&raw)
                        .ok()
                        .map(|request| callback_target(request, authority, state))
                        .transpose()?
                } else {
                    None
                };
                let code = result.flatten();
                respond(&mut socket, code.is_some());
                if let Some(code) = code {
                    return Ok(code);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err(AppError::network("授权回调监听失败")),
        }
    }
}
pub(crate) fn start(
    app: tauri::AppHandle,
    db: SharedDb,
    mut account: RealAccount,
    config: Config,
) -> Result<String, AppError> {
    config.validate()?;
    if account.name.trim().is_empty()
        || !account.email.contains('@')
        || account.auth_user.is_empty()
        || account.auth_user.len() > 1024
        || account.auth_user.chars().any(char::is_control)
    {
        return Err(AppError::invalid("请填写账号名称、邮箱及登录用户"));
    }
    let expected = {
        let conn = db.lock().map_err(|_| unavailable())?;
        account_service::existing_generation(&conn, &account)?
    };
    account.imap_host = config.host().into();
    account.imap_port = 993;
    account.imap_security = crate::imap_transport::Security::Tls;
    account.receive_only = true;
    account.oauth = Some(config.clone());
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| AppError::network("无法创建本机授权回调"))?;
    listener.set_nonblocking(true).map_err(|_| unavailable())?;
    let authority = format!(
        "{}:{}",
        config.redirect_host(),
        listener.local_addr().map_err(|_| unavailable())?.port()
    );
    let redirect = format!("http://{authority}/callback");
    let state = random();
    let verifier = random();
    let url = authorization_url(&config, &redirect, &state, &verifier, &account.auth_user)?;
    let id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut flows = flows().lock().map_err(|_| unavailable())?;
        if flows
            .values()
            .filter(|f| {
                ["waiting", "exchanging", "verifying", "committing"]
                    .contains(&f.snapshot.state.as_str())
            })
            .count()
            >= 2
        {
            return Err(AppError::conflict("已有两个授权正在进行，请先完成或取消"));
        }
        if flows.len() >= 16 {
            let oldest = flows
                .iter()
                .filter(|(_, f)| {
                    ["saved", "failed", "cancelled"].contains(&f.snapshot.state.as_str())
                })
                .min_by_key(|(_, f)| f.created)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                flows.remove(&id);
            }
        }
        flows.insert(
            id.clone(),
            Entry {
                snapshot: Snapshot {
                    id: id.clone(),
                    state: "waiting".into(),
                    account_id: None,
                    error: None,
                    folder_count: None,
                },
                cancel: cancel.clone(),
                created: Instant::now(),
            },
        );
    }
    if app.opener().open_url(url, None::<String>).is_err() {
        flows().lock().map_err(|_| unavailable())?.remove(&id);
        return Err(AppError::network("系统浏览器未能打开授权页面"));
    }
    let task = id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<(), AppError> {
            let code = wait(listener, &authority, &state, &cancel)?;
            stopped(&cancel)?;
            phase(&task, "exchanging", None, None, None)?;
            let token = token_request(
                &config,
                &[
                    ("client_id".into(), config.client_id.clone()),
                    ("grant_type".into(), "authorization_code".into()),
                    ("code".into(), code),
                    ("redirect_uri".into(), redirect),
                    ("code_verifier".into(), verifier),
                ],
            )?;
            stopped(&cancel)?;
            let refresh = token.refresh_token.ok_or_else(|| {
                AppError::auth("服务商未返回离线授权，请确认 offline_access 或重新同意授权")
            })?;
            phase(&task, "verifying", None, None, None)?;
            let tls = crate::imap_transport::connect(
                config.host(),
                993,
                crate::imap_transport::Security::Tls,
            )?;
            let mut session = login(
                crate::imap_limits::LimitedStream::new(tls),
                &account.auth_user,
                &token.access_token,
            )?;
            let count = session
                .list(Some(""), Some("*"))
                .map_err(|e| crate::imap_limits::network(e, "OAuth 登录后无法读取文件夹"))?
                .len();
            let _ = session.logout();
            stopped(&cancel)?;
            phase(&task, "committing", None, None, None)?;
            let secret = encode_secret(&Secret {
                config,
                user: account.auth_user.clone(),
                refresh_token: refresh,
            })?;
            let saved = account_service::persist(&db, account, expected, secret)?;
            phase(&task, "saved", None, Some(saved), Some(count))?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = phase(&task, "failed", Some(error.message), None, None);
        }
    });
    Ok(id)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_is_bound_to_state_host_path_and_a_single_code() {
        let good =
            "GET /callback?state=secret&code=approved HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n";
        assert_eq!(
            callback_target(good, "127.0.0.1:8080", "secret").unwrap(),
            Some("approved".into())
        );
        for request in [
            good.replace("state=secret", "state=other"),
            good.replace("Host: 127.0.0.1:8080", "Host: attacker.example"),
            good.replace("code=approved", "code=approved&code=second"),
            good.replace("/callback?", "/other?"),
        ] {
            assert!(callback_target(&request, "127.0.0.1:8080", "secret")
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn pkce_and_scopes_use_the_public_client_flow() {
        assert_eq!(
            pkce("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let config = Config {
            provider: Provider::Google,
            client_id: "public-client".into(),
            tenant: "common".into(),
        };
        let url = authorization_url(
            &config,
            "http://127.0.0.1:8080/callback",
            "state",
            "verifier",
            "me@example.test",
        )
        .unwrap();
        assert!(url.contains("code_challenge_method=S256"));
        assert!(!url.contains("client_secret"));
        assert!(url.starts_with("https://accounts.google.com/"));
    }
}
