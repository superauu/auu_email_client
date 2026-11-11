//! Public-client PKCE / XOAUTH2 support. Tokens never cross IPC or enter SQLite.
use crate::{credentials, error::AppError, mail_sync::SharedDb};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::HashMap,
    io::{Read, Write},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
#[path = "oauth_flow.rs"]
mod flow;
pub(crate) use flow::{cancel, start, status, Snapshot};
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Provider {
    Google,
    Microsoft,
}
fn common() -> String {
    "common".into()
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    pub provider: Provider,
    pub client_id: String,
    #[serde(default = "common")]
    pub tenant: String,
}
impl Config {
    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if self.client_id.is_empty()
            || self.client_id.len() > 512
            || self.client_id.chars().any(char::is_whitespace)
            || self.client_id.chars().any(char::is_control)
            || self.tenant.is_empty()
            || self.tenant.len() > 64
            || !self
                .tenant
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(AppError::invalid("OAuth client ID 或租户配置无效"));
        }
        Ok(())
    }
    pub(crate) fn host(&self) -> &'static str {
        match self.provider {
            Provider::Google => "imap.gmail.com",
            Provider::Microsoft => "outlook.office365.com",
        }
    }
    fn scope(&self) -> &'static str {
        match self.provider {
            Provider::Google => "https://mail.google.com/",
            Provider::Microsoft => {
                "https://outlook.office.com/IMAP.AccessAsUser.All offline_access"
            }
        }
    }
    fn authorization_endpoint(&self) -> String {
        match self.provider {
            Provider::Google => "https://accounts.google.com/o/oauth2/v2/auth".into(),
            Provider::Microsoft => format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
                self.tenant
            ),
        }
    }
    fn token_endpoint(&self) -> String {
        match self.provider {
            Provider::Google => "https://oauth2.googleapis.com/token".into(),
            Provider::Microsoft => format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                self.tenant
            ),
        }
    }
    fn redirect_host(&self) -> &'static str {
        match self.provider {
            Provider::Google => "127.0.0.1",
            Provider::Microsoft => "localhost",
        }
    }
}
pub(crate) fn pkce(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
    token_type: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
}
fn parse_token(raw: &[u8], config: &Config) -> Result<Token, AppError> {
    if raw.len() > 65536 {
        return Err(AppError::network("OAuth 响应超限"));
    }
    let token: Token = serde_json::from_slice(raw)
        .map_err(|_| AppError::auth("OAuth 未返回完整令牌，请重新授权"))?;
    if token.access_token.is_empty()
        || token.access_token.len() > 32768
        || token.access_token.chars().any(char::is_control)
        || !token.token_type.eq_ignore_ascii_case("Bearer")
        || token.expires_in == 0
        || token.expires_in > 86400
        || token
            .refresh_token
            .as_ref()
            .is_some_and(|t| t.is_empty() || t.len() > 16384 || t.chars().any(char::is_control))
        || token.scope.as_ref().is_some_and(|scope| {
            !scope
                .split_ascii_whitespace()
                .any(|s| s == config.scope().split_ascii_whitespace().next().unwrap_or(""))
        })
    {
        return Err(AppError::auth(
            "OAuth 令牌类型、有效期或 IMAP 权限不符合要求",
        ));
    }
    Ok(token)
}
fn token_request(config: &Config, fields: &[(String, String)]) -> Result<Token, AppError> {
    config.validate()?;
    crate::sync_control::check()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppError::network("无法初始化 OAuth TLS 连接"))?;
    let response = client
        .post(config.token_endpoint())
        .form(fields)
        .send()
        .map_err(|_| AppError::network("OAuth 服务连接失败或超时"))?;
    let status = response.status();
    let mut raw = vec![];
    response
        .take(65537)
        .read_to_end(&mut raw)
        .map_err(|_| AppError::network("OAuth 响应读取失败"))?;
    crate::sync_control::check()?;
    if !status.is_success() {
        return Err(if status.as_u16() == 429 || status.is_server_error() {
            AppError::network("OAuth 服务限流或暂不可用")
        } else {
            AppError::auth("OAuth 授权已撤销或客户端配置被拒绝，请核对配置并重新授权")
        });
    }
    parse_token(&raw, config)
}
#[derive(Deserialize, Serialize)]
struct Secret {
    config: Config,
    user: String,
    refresh_token: String,
}
const PREFIX: &str = "auumail-oauth-v1:";
fn encode_secret(secret: &Secret) -> Result<String, AppError> {
    serde_json::to_string(secret)
        .map(|s| format!("{PREFIX}{s}"))
        .map_err(|_| AppError::credentials("无法编码 OAuth 凭据"))
}
struct CachedToken {
    generation: String,
    value: String,
    expires: Instant,
}
static TOKENS: OnceLock<Mutex<HashMap<String, CachedToken>>> = OnceLock::new();
pub(crate) fn forget_token(id: &str) {
    if let Ok(mut cache) = TOKENS.get_or_init(Default::default).lock() {
        cache.remove(id);
    }
}
pub(crate) fn access(
    db: &SharedDb,
    id: &str,
    generation: &str,
    user: &str,
    config: &Config,
) -> Result<String, AppError> {
    config.validate()?;
    credentials::with_account("imap", id, || {
        let check = || -> Result<(), AppError> {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            let current: String = conn
                .query_row(
                    "SELECT sync_generation FROM accounts WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|_| AppError::conflict("原账号已移除"))?;
            if current != generation {
                return Err(AppError::conflict("账号已改变，未刷新旧授权"));
            }
            Ok(())
        };
        check()?;
        if let Some(token) = TOKENS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| AppError::credentials("OAuth 缓存不可用"))?
            .get(id)
            .filter(|t| t.generation == generation && t.expires > Instant::now())
        {
            return Ok(token.value.clone());
        }
        let encoded = credentials::secret_for("imap", id)?;
        let mut secret: Secret = encoded
            .strip_prefix(PREFIX)
            .and_then(|s| serde_json::from_str(s).ok())
            .ok_or_else(|| AppError::credentials("OAuth 凭据缺失，请重新授权"))?;
        if secret.config != *config || secret.user != user {
            return Err(AppError::auth("授权配置与账号不一致，请重新授权"));
        }
        let token = token_request(
            config,
            &[
                ("client_id".into(), config.client_id.clone()),
                ("grant_type".into(), "refresh_token".into()),
                ("refresh_token".into(), secret.refresh_token.clone()),
                ("scope".into(), config.scope().into()),
            ],
        )?;
        check()?;
        if let Some(refresh) = token.refresh_token {
            if refresh != secret.refresh_token {
                secret.refresh_token = refresh;
                credentials::set("imap", id, &encode_secret(&secret)?)?;
            }
        }
        let value = token.access_token;
        let mut cache = TOKENS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| AppError::credentials("OAuth 缓存不可用"))?;
        cache.retain(|_, t| t.expires > Instant::now());
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(
            id.into(),
            CachedToken {
                generation: generation.into(),
                value: value.clone(),
                expires: Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(60)),
            },
        );
        Ok(value)
    })
}
struct Xoauth<'a> {
    user: &'a str,
    token: &'a str,
    sent: Cell<bool>,
}
impl imap::Authenticator for Xoauth<'_> {
    type Response = Vec<u8>;
    fn process(&self, challenge: &[u8]) -> Self::Response {
        if self.sent.replace(true) || !challenge.is_empty() {
            vec![]
        } else {
            format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token).into_bytes()
        }
    }
}
pub(crate) fn login<S: Read + Write>(
    stream: S,
    user: &str,
    token: &str,
) -> Result<imap::Session<S>, AppError> {
    if user.is_empty()
        || user.len() > 1024
        || user.chars().any(char::is_control)
        || token.is_empty()
        || token.chars().any(char::is_control)
    {
        return Err(AppError::invalid("XOAUTH2 用户或令牌无效"));
    }
    imap::Client::new(stream)
        .authenticate(
            "XOAUTH2",
            &Xoauth {
                user,
                token,
                sent: Cell::new(false),
            },
        )
        .map_err(|(e, _)| match e {
            imap::Error::No(_) | imap::Error::Bad(_) => {
                AppError::auth("IMAP OAuth 登录被拒绝，请核对邮箱权限或重新授权")
            }
            e => crate::imap_limits::network(e, "IMAP OAuth 会话无法建立"),
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_validation_and_sasl_errors_never_echo_credentials() {
        let config = Config {
            provider: Provider::Google,
            client_id: "public-id".into(),
            tenant: "common".into(),
        };
        let raw = serde_json::json!({"access_token":"private-access","token_type":"Bearer","expires_in":3600,"refresh_token":"private-refresh","scope":"https://mail.google.com/"});
        assert!(parse_token(raw.to_string().as_bytes(), &config).is_ok());
        for (key, value) in [
            ("scope", serde_json::json!("email")),
            ("token_type", serde_json::json!("Basic")),
            ("expires_in", serde_json::json!(0)),
            ("access_token", serde_json::json!("private\r\naccess")),
        ] {
            let mut bad = raw.clone();
            bad[key] = value;
            let error = parse_token(bad.to_string().as_bytes(), &config)
                .err()
                .unwrap();
            assert!(!error.message.contains("private"));
        }
        assert!(parse_token(&vec![b' '; 65537], &config).is_err());
        let auth = Xoauth {
            user: "user@example.test",
            token: "private-access",
            sent: Cell::new(false),
        };
        use imap::Authenticator;
        assert_eq!(
            auth.process(b""),
            b"user=user@example.test\x01auth=Bearer private-access\x01\x01"
        );
        assert!(auth.process(b"error challenge").is_empty());
        assert!(auth.process(b"").is_empty());
        assert!(login(
            std::io::Cursor::new(Vec::<u8>::new()),
            "bad\x01user",
            "private"
        )
        .is_err());
        let mut bad = config;
        bad.tenant = "../../evil".into();
        assert!(bad.validate().is_err());
    }
}
