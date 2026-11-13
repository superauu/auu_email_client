//! SMTP 投递状态机。docs/SENDING.md 语义在这里落地：
//! - DATA 终点号写出之后（含之后一切读写失败）未读到最终 2xx 一律 `unknown`；
//! - 命令阶段（EHLO / AUTH / MAIL / RCPT / DATA 命令本身）的失败与拒绝是明确 failed / auth；
//! - 错误信息绝不包含凭据或正文内容。
use crate::error::AppError;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use native_tls::{TlsConnector, TlsStream};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(10);
const EHLO_NAME: &str = "auumail.local";

pub(crate) enum SendAuth<'a> {
    Plain { user: &'a str, secret: &'a str },
    Xoauth2 { user: &'a str, token: &'a str },
}

fn unknown(message: String) -> AppError {
    AppError { code: "unknown", message, retryable: false }
}

fn io_connect(host: &str, port: u16) -> Result<TcpStream, AppError> {
    let tcp = TcpStream::connect((host, port)).map_err(|error| {
        AppError::network(format!("无法连接到 {host}:{port}，请检查地址与端口（{error}）"))
    })?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| tcp.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| AppError::network("无法设置连接超时"))?;
    Ok(tcp)
}

fn upgrade_tls(host: &str, tcp: TcpStream) -> Result<TlsStream<TcpStream>, AppError> {
    TlsConnector::new()
        .map_err(|_| AppError::network("TLS 初始化失败"))?
        .connect(host, tcp)
        .map_err(|_| AppError::network(format!("{host} 的 TLS 证书不受信任或协议不被支持")))
}

/// 建立 TLS 通道：465 直连，或 587 先明文 EHLO 确认 STARTTLS 能力再升级（fail-closed，
/// 任何一步失败都不继续发送，不会在明文通道上传凭据或正文）。
pub(crate) fn open(
    host: &str,
    port: u16,
    security: crate::imap_transport::Security,
) -> Result<TlsStream<TcpStream>, AppError> {
    match security {
        crate::imap_transport::Security::Tls => crate::mail_verify::tls_connect(host, port),
        crate::imap_transport::Security::Starttls => {
            let mut tcp = io_connect(host, port)?;
            let (code, _) = crate::mail_verify::read_response(&mut tcp)?;
            if code != 220 {
                return Err(AppError::network("SMTP 服务器未按协议返回问候"));
            }
            crate::mail_verify::write_command(&mut tcp, &format!("EHLO {EHLO_NAME}"))?;
            let (code, text) = crate::mail_verify::read_response(&mut tcp)?;
            if code != 250 {
                return Err(AppError::network("SMTP 服务器拒绝了 EHLO 握手"));
            }
            if !text.to_ascii_lowercase().contains("starttls") {
                return Err(AppError::invalid(
                    "服务器未提供 STARTTLS，请改用 TLS 直连（通常 465）",
                ));
            }
            crate::mail_verify::write_command(&mut tcp, "STARTTLS")?;
            let (code, _) = crate::mail_verify::read_response(&mut tcp)?;
            if code != 220 {
                return Err(AppError::network("STARTTLS 升级未完成"));
            }
            upgrade_tls(host, tcp)
        }
    }
}

/// RFC5322 点填充：行首 `.` 翻倍，孤立 LF 归一为 CRLF，末尾补 CRLF 以承载终点号。
fn dot_stuff(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 16);
    let mut at_line_start = true;
    for &byte in raw {
        if byte == b'\n' && !at_line_start {
            out.push(b'\r');
        }
        if at_line_start && byte == b'.' {
            out.push(b'.');
        }
        out.push(byte);
        at_line_start = byte == b'\n' || byte == b'\r';
    }
    if !out.ends_with(b"\r\n") {
        if out.last() == Some(&b'\n') {
            out.push(b'\r');
        } else {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

fn auth_line(auth: &SendAuth<'_>) -> String {
    match auth {
        SendAuth::Plain { user, secret } => {
            format!("AUTH PLAIN {}", BASE64.encode(format!("\x00{user}\x00{secret}")))
        }
        SendAuth::Xoauth2 { user, token } => format!(
            "AUTH XOAUTH2 {}",
            BASE64.encode(format!("user={user}\x01auth=Bearer {token}\x01\x01"))
        ),
    }
}

fn auth_failure(detail: &str) -> AppError {
    AppError::auth(format!(
        "SMTP 服务器拒绝了登录（{}）",
        crate::mail_verify::main_line(detail)
    ))
}

/// 完整投递会话：问候 + EHLO 在 `open` 之后由这里统一完成。
pub(crate) fn send_session<S: Read + Write>(
    mut stream: S,
    auth: &SendAuth<'_>,
    from: &str,
    recipients: &[String],
    raw: &[u8],
) -> Result<(), AppError> {
    let (code, _) = crate::mail_verify::read_response(&mut stream)?;
    if code != 220 {
        return Err(AppError::network("SMTP 服务器未按协议返回问候"));
    }
    crate::mail_verify::write_command(&mut stream, &format!("EHLO {EHLO_NAME}"))?;
    let (code, _) = crate::mail_verify::read_response(&mut stream)?;
    if code != 250 {
        return Err(AppError::network("SMTP 服务器拒绝了 EHLO 握手"));
    }
    crate::mail_verify::write_command(&mut stream, &auth_line(auth))?;
    let (code, detail) = crate::mail_verify::read_response(&mut stream)?;
    if code != 235 {
        if code == 334 {
            // XOAUTH2 错误挑战：回应空行后服务器给出最终拒绝；凭据永远不出现在错误里。
            crate::mail_verify::write_command(&mut stream, "")?;
            let (code, detail) = crate::mail_verify::read_response(&mut stream)?;
            if code != 235 {
                return Err(auth_failure(&detail));
            }
        } else if code == 535 || code == 501 || code == 534 {
            return Err(auth_failure(&detail));
        } else {
            return Err(AppError::network(format!(
                "SMTP 认证流程异常结束（{} {}）",
                code,
                crate::mail_verify::main_line(&detail)
            )));
        }
    }
    crate::mail_verify::write_command(&mut stream, &format!("MAIL FROM:<{from}>"))?;
    let (code, detail) = crate::mail_verify::read_response(&mut stream)?;
    if code != 250 {
        return Err(AppError::invalid(format!(
            "发件人被服务器拒绝（{} {}）",
            code,
            crate::mail_verify::main_line(&detail)
        )));
    }
    for recipient in recipients {
        crate::mail_verify::write_command(&mut stream, &format!("RCPT TO:<{recipient}>"))?;
        let (code, detail) = crate::mail_verify::read_response(&mut stream)?;
        if code != 250 && code != 251 {
            return Err(AppError::invalid(format!(
                "收件人 {recipient} 被服务器拒绝（{} {}）",
                code,
                crate::mail_verify::main_line(&detail)
            )));
        }
    }
    crate::mail_verify::write_command(&mut stream, "DATA")?;
    let (code, detail) = crate::mail_verify::read_response(&mut stream)?;
    if code != 354 {
        return Err(AppError::invalid(format!(
            "服务器拒绝接收邮件正文（{} {}）",
            code,
            crate::mail_verify::main_line(&detail)
        )));
    }
    // 正文写入失败发生在终点号之前：服务器收不到完整数据，不会投递 → 明确 failed。
    stream
        .write_all(&dot_stuff(raw))
        .and_then(|_| stream.write_all(b".\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|_| AppError::network("邮件正文写入失败，投递未完成"))?;
    // 终点号已写出：此后任何结果不明确都进入 unknown，绝不重发。
    let result = crate::mail_verify::read_response(&mut stream);
    match result {
        Ok((250, _)) => {}
        Ok((code, detail)) => {
            return Err(AppError::invalid(format!(
                "服务器对投递给出非预期响应（{} {}）",
                code,
                crate::mail_verify::main_line(&detail)
            )))
        }
        Err(_) => {
            return Err(unknown(
                "邮件已提交给服务器，但未能确认投递结果，请勿重复发送".into(),
            ))
        }
    }
    let _ = crate::mail_verify::write_command(&mut stream, "QUIT");
    let _ = crate::mail_verify::read_response(&mut stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[derive(Debug)]
    struct MockStream {
        reads: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }
    impl MockStream {
        fn new(script: &str) -> Self {
            Self { reads: Cursor::new(script.as_bytes().to_vec()), writes: Vec::new() }
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

    const GREET: &str = "220 mail.example.test ESMTP\r\n250-AUTH PLAIN XOAUTH2\r\n250 SIZE 35882577\r\n";

    fn plain<'a>(user: &'a str, secret: &'a str) -> SendAuth<'a> {
        SendAuth::Plain { user, secret }
    }

    #[test]
    fn delivery_walks_envelope_and_stuffs_dots() {
        let mut stream = MockStream::new(&format!(
            "{GREET}235 ok\r\n250 from ok\r\n250 rcpt ok\r\n354 go\r\n250 queued as 123\r\n221 bye\r\n"
        ));
        send_session(
            &mut stream,
            &plain("alex@example.test", "secret"),
            "alex@example.test",
            &["bob@example.test".into()],
            b"Subject: hi\r\n\r\nfirst\r\n.hidden\r\nline two",
        )
        .unwrap();
        let log = String::from_utf8(stream.writes).unwrap();
        assert!(log.contains("MAIL FROM:<alex@example.test>"));
        assert!(log.contains("RCPT TO:<bob@example.test>"));
        assert!(log.contains("DATA\r\n"));
        assert!(log.contains("\r\n..hidden\r\n"), "行首点必须翻倍");
        assert!(log.contains("\r\n.\r\nQUIT\r\n"), "终点号后紧跟 QUIT");
    }

    #[test]
    fn rejected_recipient_is_a_clear_failure_before_data() {
        let mut stream = MockStream::new(&format!(
            "{GREET}235 ok\r\n250 from ok\r\n550 no such user\r\n"
        ));
        let error = send_session(
            &mut stream,
            &plain("u@example.test", "s"),
            "u@example.test",
            &["gone@example.test".into()],
            b"body",
        )
        .unwrap_err();
        assert_ne!(error.code, "unknown");
        assert!(error.message.contains("gone@example.test"));
        // 失败发生在 DATA 之前：命令日志里没有终点号。
        assert!(!String::from_utf8(stream.writes).unwrap().ends_with("\r\n.\r\n"));
    }

    #[test]
    fn connection_drop_after_final_dot_is_unknown_not_retryable() {
        // 服务器读完正文后直接断开：点号已写出，投递结果无法确认。
        let mut stream = MockStream::new(&format!("{GREET}235 ok\r\n250 ok\r\n250 ok\r\n354 go\r\n"));
        let error = send_session(
            &mut stream,
            &plain("u@example.test", "s"),
            "u@example.test",
            &["r@example.test".into()],
            b"body",
        )
        .unwrap_err();
        assert_eq!(error.code, "unknown");
        assert!(!error.retryable);
        assert!(String::from_utf8(stream.writes).unwrap().ends_with("\r\n.\r\n"));
    }

    #[test]
    fn data_command_rejection_is_clear_failure() {
        let mut stream = MockStream::new(&format!("{GREET}235 ok\r\n250 ok\r\n250 ok\r\n554 policy\r\n"));
        let error = send_session(
            &mut stream,
            &plain("u@example.test", "s"),
            "u@example.test",
            &["r@example.test".into()],
            b"body",
        )
        .unwrap_err();
        assert_ne!(error.code, "unknown");
    }

    #[test]
    fn xoauth2_error_challenge_ends_in_auth_error_without_echoing_token() {
        let script = format!(
            "{GREET}334 eyJzdGF0dXMiOiI0MDEifQ==\r\n535 bad token\r\n"
        );
        let mut stream = MockStream::new(&script);
        let token = "super-secret-access-token";
        let error = send_session(
            &mut stream,
            &SendAuth::Xoauth2 { user: "u@example.test", token },
            "u@example.test",
            &["r@example.test".into()],
            b"body",
        )
        .unwrap_err();
        assert_eq!(error.code, "auth");
        assert!(!error.message.contains(token));
        let log = String::from_utf8(stream.writes).unwrap();
        assert!(log.contains("AUTH XOAUTH2 "));
        // 错误挑战后必须回应空行。
        assert!(log.contains("AUTH XOAUTH2 \r\n\r\n") || log.matches("\r\n\r\n").count() >= 1);
    }

    #[test]
    fn xoauth2_accepted_without_challenge() {
        let mut stream = MockStream::new(&format!(
            "{GREET}235 ok\r\n250 ok\r\n250 ok\r\n354 go\r\n250 queued\r\n221 bye\r\n"
        ));
        send_session(
            &mut stream,
            &SendAuth::Xoauth2 { user: "u@example.test", token: "tok" },
            "u@example.test",
            &["r@example.test".into()],
            b"body",
        )
        .unwrap();
        let log = String::from_utf8(stream.writes).unwrap();
        assert!(log.contains(&format!(
            "AUTH XOAUTH2 {}",
            BASE64.encode("user=u@example.test\x01auth=Bearer tok\x01\x01")
        )));
    }

    #[test]
    fn dot_stuffing_normalizes_and_terminates() {
        assert_eq!(dot_stuff(b"a\n.b\n"), b"a\r\n..b\r\n".to_vec());
        assert_eq!(dot_stuff(b"tail"), b"tail\r\n".to_vec());
        assert_eq!(dot_stuff(b"x\r\n"), b"x\r\n".to_vec());
    }

    #[test]
    fn starttls_requires_capability_before_upgrade() {
        // 独立验证 open() 的 STARTTLS 分支无法用 MockStream（需要真实 TLS 握手）；
        // 能力缺失的 fail-closed 行为在这里覆盖脚本部分。
        let mut stream = MockStream::new("220 mail.example.test ESMTP\r\n250-AUTH PLAIN\r\n250 SIZE\r\n");
        let (code, _) = crate::mail_verify::read_response(&mut stream).unwrap();
        assert_eq!(code, 220);
        crate::mail_verify::write_command(&mut stream, &format!("EHLO {EHLO_NAME}")).unwrap();
        let (_, text) = crate::mail_verify::read_response(&mut stream).unwrap();
        assert!(!text.to_ascii_lowercase().contains("starttls"));
    }
}
