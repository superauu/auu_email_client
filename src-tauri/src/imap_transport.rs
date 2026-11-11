//! Mandatory TLS, including a bounded, credential-free STARTTLS negotiation.
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    #[default]
    Tls,
    Starttls,
}

fn line<S: Read>(stream: &mut S, budget: &mut usize, until: Instant) -> Result<String, AppError> {
    let mut out = Vec::new();
    loop {
        crate::sync_control::check()?;
        if Instant::now() >= until {
            return Err(AppError::network("STARTTLS 握手超时"));
        }
        let mut byte = [0];
        if stream
            .read(&mut byte)
            .map_err(|_| AppError::network("STARTTLS 握手读取失败"))?
            != 1
        {
            return Err(AppError::network("STARTTLS 握手提前断开"));
        }
        *budget += 1;
        if *budget > 65536 || out.len() >= 8192 {
            return Err(AppError::network("STARTTLS 握手响应超限"));
        }
        out.push(byte[0]);
        if byte[0] == b'\n' {
            if !out.ends_with(b"\r\n") {
                return Err(AppError::network("STARTTLS 响应格式错误"));
            }
            return String::from_utf8(out).map_err(|_| AppError::network("STARTTLS 响应编码错误"));
        }
    }
}
fn command<S: Read + Write>(
    stream: &mut S,
    text: &[u8],
    tag: &str,
    budget: &mut usize,
    until: Instant,
) -> Result<Vec<String>, AppError> {
    stream
        .write_all(text)
        .and_then(|_| stream.flush())
        .map_err(|_| AppError::network("STARTTLS 握手写入失败"))?;
    let mut result = vec![];
    loop {
        let response = line(stream, budget, until)?;
        let mut words = response.split_ascii_whitespace();
        let first = words.next();
        if first == Some(tag) {
            if !words
                .next()
                .is_some_and(|status| status.eq_ignore_ascii_case("OK"))
            {
                return Err(AppError::network("服务器拒绝 STARTTLS，未发送凭据"));
            }
            return Ok(result);
        }
        if first != Some("*") || words.next().is_some_and(|w| w.eq_ignore_ascii_case("BYE")) {
            return Err(AppError::network("STARTTLS 响应次序错误"));
        }
        result.push(response);
    }
}
fn negotiate<S: Read + Write>(stream: &mut S) -> Result<(), AppError> {
    let mut budget = 0;
    let until = Instant::now() + Duration::from_secs(30);
    let greeting = line(stream, &mut budget, until)?;
    if !greeting.to_ascii_uppercase().starts_with("* OK ") {
        return Err(AppError::network("IMAP 未返回有效问候，未发送凭据"));
    }
    let capabilities = command(
        stream,
        b"auucap CAPABILITY\r\n",
        "auucap",
        &mut budget,
        until,
    )?;
    let supported = capabilities.iter().any(|line| {
        let mut words = line.split_ascii_whitespace();
        words.next() == Some("*")
            && words
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("CAPABILITY"))
            && words.any(|w| w.eq_ignore_ascii_case("STARTTLS"))
    });
    if !supported {
        return Err(AppError::network("服务器未声明 STARTTLS，禁止降级明文登录"));
    }
    command(stream, b"auutls STARTTLS\r\n", "auutls", &mut budget, until)?;
    Ok(())
}
pub(crate) fn connect(
    host: &str,
    port: u16,
    security: Security,
) -> Result<native_tls::TlsStream<TcpStream>, AppError> {
    crate::sync_control::check()?;
    if host.is_empty()
        || host.len() > 253
        || host.chars().any(|c| c.is_whitespace() || c.is_control())
        || port == 0
    {
        return Err(AppError::invalid("IMAP 地址或端口无效"));
    }
    let until = Instant::now() + Duration::from_secs(20);
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| AppError::network("无法解析 IMAP 地址"))?;
    crate::sync_control::check()?;
    let mut socket = None;
    for address in addresses.take(8) {
        crate::sync_control::check()?;
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Ok(stream) =
            TcpStream::connect_timeout(&address, remaining.min(Duration::from_secs(5)))
        {
            socket = Some(stream);
            break;
        }
    }
    let mut socket = socket.ok_or_else(|| AppError::network("IMAP 连接失败或超时"))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .and_then(|_| socket.set_write_timeout(Some(Duration::from_secs(10))))
        .map_err(|_| AppError::network("无法设置 IMAP 超时"))?;
    if security == Security::Starttls {
        negotiate(&mut socket)?;
    }
    crate::sync_control::check()?;
    let tls = native_tls::TlsConnector::new()
        .map_err(|_| AppError::network("TLS 初始化失败"))?
        .connect(host, socket)
        .map_err(|_| AppError::network("IMAP TLS 证书不受信任、主机名不符或握手失败"))?;
    crate::sync_control::check()?;
    Ok(tls)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Script {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl Read for Script {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(b)
        }
    }
    impl Write for Script {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn starttls_requires_capability_and_success_without_sending_credentials_or_overreading() {
        let response = b"* OK ready\r\n* CAPABILITY IMAP4rev1 STARTTLS\r\nauucap OK capabilities\r\nauutls OK upgrade\r\nTLS-BYTES";
        let mut stream = Script {
            input: std::io::Cursor::new(response.to_vec()),
            output: vec![],
        };
        negotiate(&mut stream).unwrap();
        assert_eq!(stream.output, b"auucap CAPABILITY\r\nauutls STARTTLS\r\n");
        assert_eq!(&response[stream.input.position() as usize..], b"TLS-BYTES");
    }
    #[test]
    fn starttls_refusal_missing_support_bad_greeting_and_oversized_response_fail_closed() {
        for response in [
            "* PREAUTH unsafe\r\n".into(),
            "* OK ready\r\n* CAPABILITY IMAP4rev1\r\nauucap OK caps\r\n".into(),
            "* OK ready\r\n* CAPABILITY STARTTLS\r\nauucap OK caps\r\nauutls NO refused\r\n".into(),
            format!("* OK {}", "x".repeat(8192)),
        ] {
            let mut stream = Script {
                input: std::io::Cursor::new(response.into_bytes()),
                output: vec![],
            };
            assert!(negotiate(&mut stream).is_err());
            assert!(!String::from_utf8(stream.output).unwrap().contains("LOGIN"));
        }
    }
}
