//! IMAP / SMTP connection verification for the account wizard.
//! Both probes are honest: results only ever come from a real protocol conversation.
//! Secrets never appear in error messages or logs.
use crate::error::AppError;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use native_tls::TlsConnector;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const EHLO_NAME: &str = "auumail.local";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOutcome {
    pub ok: bool,
    pub error: Option<String>,
    /// "network" or "auth"; lets the wizard point at the right fix.
    pub code: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub imap: ProbeOutcome,
    pub smtp: ProbeOutcome,
    /// Real folder count returned by IMAP LIST; None when IMAP never succeeded.
    pub folder_count: Option<usize>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoints {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub auth_user: String,
}

fn sanitize_host(host: &str) -> Result<&str, AppError> {
    let host = host.trim();
    if host.is_empty()
        || host.len() > 253
        || host.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(AppError::invalid("服务器地址不符合要求"));
    }
    Ok(host)
}

/// Runs both probes independently so one broken endpoint does not hide the other's result.
pub fn verify(endpoints: &Endpoints, secret: &str) -> VerifyReport {
    let imap = imap_probe(
        &endpoints.imap_host,
        endpoints.imap_port,
        &endpoints.auth_user,
        secret,
    );
    let smtp = smtp_probe(
        &endpoints.smtp_host,
        endpoints.smtp_port,
        &endpoints.auth_user,
        secret,
    );
    let (imap_ok, imap_error, imap_code, folder_count) = match imap {
        Ok(count) => (true, None, None, Some(count)),
        Err(error) => (false, Some(error.message), Some(error.code), None),
    };
    let (smtp_ok, smtp_error, smtp_code) = match smtp {
        Ok(()) => (true, None, None),
        Err(error) => (false, Some(error.message), Some(error.code)),
    };
    VerifyReport {
        imap: ProbeOutcome {
            ok: imap_ok,
            error: imap_error,
            code: imap_code,
        },
        smtp: ProbeOutcome {
            ok: smtp_ok,
            error: smtp_error,
            code: smtp_code,
        },
        folder_count,
    }
}

/// Gate for saving: every probe must have passed, otherwise the combined failure is returned.
pub fn ensure_verified(report: &VerifyReport) -> Result<(), AppError> {
    let mut failures = Vec::new();
    let mut code = "network";
    for (label, probe) in [("IMAP", &report.imap), ("SMTP", &report.smtp)] {
        match (&probe.error, probe.code) {
            (Some(error), Some(kind)) => {
                if kind == "auth" {
                    code = "auth";
                }
                failures.push(format!("{label}：{error}"));
            }
            (Some(error), None) => failures.push(format!("{label}：{error}")),
            (None, _) => {}
        }
    }
    if failures.is_empty() {
        Ok(())
    } else if code == "auth" {
        Err(AppError::auth(failures.join("；")))
    } else {
        Err(AppError::network(failures.join("；")))
    }
}

/// TLS login + LIST: returns the real folder count on success.
fn imap_probe(host: &str, port: u16, user: &str, secret: &str) -> Result<usize, AppError> {
    let host = sanitize_host(host)?;
    let tls = tls_connect(host, port)?;
    let client = imap::Client::new(tls);
    let mut session = client
        .login(user, secret)
        .map_err(|(error, _)| match error {
            imap::Error::No(_) | imap::Error::Bad(_) => {
                AppError::auth("IMAP 服务器拒绝了登录，请核对登录用户与密码")
            }
            _ => AppError::network("IMAP 会话未能建立"),
        })?;
    let folders = session
        .list(Some(""), Some("*"))
        .map_err(|_| AppError::network("无法读取邮箱文件夹列表"))?;
    let count = folders.len();
    let _ = session.logout();
    Ok(count)
}

fn smtp_probe(host: &str, port: u16, user: &str, secret: &str) -> Result<(), AppError> {
    let host = sanitize_host(host)?;
    let tls = tls_connect(host, port)?;
    smtp_verify_stream(tls, user, secret)
}

fn tls_connect(host: &str, port: u16) -> Result<native_tls::TlsStream<TcpStream>, AppError> {
    let address = (host, port);
    let tcp = TcpStream::connect(address).map_err(|error| {
        AppError::network(format!(
            "无法连接到 {host}:{port}，请检查地址与端口（{error}）"
        ))
    })?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| AppError::network("无法设置连接超时"))?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| AppError::network("无法设置连接超时"))?;
    let connector = TlsConnector::new().map_err(|_| AppError::network("TLS 初始化失败"))?;
    connector
        .connect(host, tcp)
        .map_err(|_| AppError::network(format!("{host}:{port} 的 TLS 证书不受信任或协议不被支持")))
}

/// SMTP state machine over any stream so tests can script a server without a network.
fn smtp_verify_stream<S: Read + Write>(
    mut stream: S,
    user: &str,
    secret: &str,
) -> Result<(), AppError> {
    let (code, _) = read_response(&mut stream)?;
    if code != 220 {
        return Err(AppError::network("SMTP 服务器未按协议返回问候"));
    }
    write_command(&mut stream, &format!("EHLO {EHLO_NAME}"))?;
    let (code, text) = read_response(&mut stream)?;
    if code != 250 {
        return Err(AppError::network("SMTP 服务器拒绝了 EHLO 握手"));
    }
    let _ = text;
    let initial = BASE64.encode(format!("\x00{user}\x00{secret}"));
    write_command(&mut stream, &format!("AUTH PLAIN {initial}"))?;
    let (code, detail) = read_response(&mut stream)?;
    if code != 235 {
        if code == 535 || code == 501 || code == 534 {
            return Err(AppError::auth(
                "SMTP 服务器拒绝了登录，请核对登录用户与密码",
            ));
        }
        return Err(AppError::network(format!(
            "SMTP 认证流程异常结束（{} {}）",
            code,
            main_line(&detail)
        )));
    }
    write_command(&mut stream, "QUIT")?;
    let _ = read_response(&mut stream)?; // 221; after a successful auth the result no longer matters
    Ok(())
}

fn main_line(detail: &str) -> String {
    detail.lines().next().unwrap_or("").trim().to_string()
}

fn write_command<S: Write>(stream: &mut S, command: &str) -> Result<(), AppError> {
    stream
        .write_all(command.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|_| AppError::network("向 SMTP 服务器写入命令失败"))
}

/// Reads a possibly-multiline reply, returning the final status code and the full text.
/// A reply ends at the first line whose 4th byte is a space after three digits.
fn read_response<R: Read>(reader: &mut R) -> Result<(u16, String), AppError> {
    let mut text: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Err(AppError::network("连接在响应完成前关闭")),
            Ok(_) => {
                text.push(byte[0]);
                line.push(byte[0]);
                if text.len() > MAX_RESPONSE_BYTES {
                    return Err(AppError::network("SMTP 响应超出允许长度"));
                }
                if byte[0] == b'\n' {
                    if line.len() >= 4
                        && line[3] == b' '
                        && line[..3].iter().all(|b| b.is_ascii_digit())
                    {
                        let code: u16 = std::str::from_utf8(&line[..3])
                            .map_err(|_| AppError::network("无法解析 SMTP 状态码"))?
                            .parse()
                            .map_err(|_| AppError::network("无法解析 SMTP 状态码"))?;
                        return Ok((code, String::from_utf8_lossy(&text).into_owned()));
                    }
                    line.clear();
                }
            }
            Err(_) => return Err(AppError::network("读取 SMTP 响应失败（连接超时或中断）")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A scripted server: serves `reads` bytes, records everything we send.
    #[derive(Debug)]
    struct MockStream {
        reads: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }
    impl MockStream {
        fn new(script: &str) -> Self {
            Self {
                reads: Cursor::new(script.as_bytes().to_vec()),
                writes: Vec::new(),
            }
        }
    }
    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            Read::read(&mut self.reads, buf)
        }
    }
    impl Write for MockStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn smtp_happy_path_authenticates_and_says_bye() {
        let mut stream = MockStream::new("220 mail.example.test ESMTP\r\n250-mail.example.test\r\n250-SIZE 35882577\r\n250 AUTH PLAIN\r\n235 Authentication successful\r\n221 Bye\r\n");
        smtp_verify_stream(&mut stream, "alex@example.test", "secret-pass").unwrap();
        let log = String::from_utf8(stream.writes).unwrap();
        assert!(log.contains("EHLO auumail.local"));
        let expected = BASE64.encode("\x00alex@example.test\x00secret-pass");
        assert!(log.contains(&format!("AUTH PLAIN {expected}")));
        assert!(log.ends_with("QUIT\r\n"));
    }

    #[test]
    fn smtp_rejected_credentials_map_to_auth_error() {
        let mut stream = MockStream::new("220 mail.example.test ESMTP\r\n250 AUTH PLAIN\r\n535 Authentication credentials invalid\r\n");
        let error = smtp_verify_stream(&mut stream, "alex@example.test", "wrong").unwrap_err();
        assert_eq!(error.code, "auth");
        assert!(!error.message.contains("wrong"));
    }

    #[test]
    fn smtp_protocol_garbage_maps_to_network_error() {
        let mut stream = MockStream::new("999X not a status line\r\n");
        let error = smtp_verify_stream(&mut stream, "u", "p").unwrap_err();
        assert_eq!(error.code, "network");
        let mut stream = MockStream::new("");
        let error = smtp_verify_stream(&mut stream, "u", "p").unwrap_err();
        assert_eq!(error.code, "network");
    }

    /// IMAP through the real `imap` crate against a scripted in-memory server.
    #[test]
    fn imap_probe_counts_real_folders_from_scripted_server() {
        let script = "* OK IMAP4rev1 server ready\r\n\
                      a1 OK Logged in\r\n\
                      * LIST () \"/\" INBOX\r\n\
                      * LIST () \"/\" Sent\r\n\
                      * LIST () \"/\" Archive/2026\r\n\
                      a2 OK LIST completed\r\n\
                      * BYE Logging out\r\n\
                      a3 OK Logout completed\r\n";
        let client = imap::Client::new(MockStream::new(script));
        let mut session = client.login("alex@example.test", "secret-pass").unwrap();
        let folders = session.list(Some(""), Some("*")).unwrap();
        assert_eq!(folders.len(), 3);
        session.logout().unwrap();
    }

    #[test]
    fn imap_rejected_login_maps_to_auth_error() {
        let script = "* OK IMAP4rev1 server ready\r\n\
                      a1 NO [AUTHENTICATIONFAILED] Login failed\r\n";
        let client = imap::Client::new(MockStream::new(script));
        let result = client
            .login("alex@example.test", "wrong")
            .map_err(|(error, _)| match error {
                imap::Error::No(_) | imap::Error::Bad(_) => AppError::auth("IMAP 服务器拒绝了登录"),
                _ => AppError::network("IMAP 会话未能建立"),
            });
        assert_eq!(result.unwrap_err().code, "auth");
    }

    #[test]
    fn host_validation_rejects_garbage() {
        assert_eq!(sanitize_host("  ").unwrap_err().code, "invalid");
        assert_eq!(
            sanitize_host("host example.com").unwrap_err().code,
            "invalid"
        );
        assert_eq!(
            sanitize_host("mail.example.com").unwrap(),
            "mail.example.com"
        );
    }

    #[test]
    fn full_verify_reports_each_endpoint_independently() {
        // Both probes against a nonexistent local port must fail honestly, each with its own result.
        let endpoints = Endpoints {
            imap_host: "127.0.0.1".into(),
            imap_port: 1,
            smtp_host: "127.0.0.1".into(),
            smtp_port: 1,
            auth_user: "u".into(),
        };
        let report = verify(&endpoints, "unused-secret");
        assert!(!report.imap.ok);
        assert!(!report.smtp.ok);
        assert!(report.folder_count.is_none());
    }
}
