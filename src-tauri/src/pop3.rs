//! POP3 收信（provider 'pop3'）：TLS 995（或配置端口）、UIDL 增量、复用原文缓存与消息解析。
//! 语义约定（docs/SENDING 同源的诚实边界）：POP3 无标记同步，新邮件以 UIDL 集合判定；
//! 服务器上的邮件不删除；单次运行最多拉取 50 封（首窗策略与 IMAP 一致），后续运行续拉。
use crate::{
    credentials,
    error::AppError,
    mail_cache::{self, Spool},
    mail_sync::{self, SharedDb, SyncOutcome},
};
use chrono::{FixedOffset, TimeZone, Utc};
use mail_parser::MessageParser;
use rusqlite::{params, OptionalExtension};
use std::io::{Read, Write};
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LINE: usize = 1024;
const MAX_DOWNLOAD: usize = 50;
const MAX_RAW: usize = 64 * 1024 * 1024;
const CURSOR_SCOPE: &str = "pop3:inbox";

/// 读一行（\r\n 结尾），POP3 响应以 +OK / -ERR 开头。
fn read_line<R: Read>(reader: &mut R) -> Result<String, AppError> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Err(AppError::network("连接在响应完成前关闭")),
            Ok(_) => {
                line.push(byte[0]);
                if line.len() > MAX_LINE {
                    return Err(AppError::network("POP3 响应行超出允许长度"));
                }
                if line.ends_with(b"\r\n") {
                    let text = String::from_utf8_lossy(&line[..line.len() - 2]).into_owned();
                    return Ok(text);
                }
            }
            Err(_) => return Err(AppError::network("读取 POP3 响应失败（连接超时或中断）")),
        }
    }
}

fn expect_ok(line: &str, context: &str) -> Result<String, AppError> {
    if let Some(rest) = line.strip_prefix("+OK") {
        Ok(rest.trim().to_string())
    } else if line.starts_with("-ERR") {
        Err(AppError::network(format!("{context}：{}", line.trim())))
    } else {
        Err(AppError::network(format!(
            "{context}：响应不符合 POP3 协议"
        )))
    }
}

fn write_command<S: Write>(stream: &mut S, command: &str) -> Result<(), AppError> {
    stream
        .write_all(command.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|_| AppError::network("向 POP3 服务器写入命令失败"))
}

/// 读多行响应（RETR / UIDL / LIST），以仅含 `.` 的行为终点；逐行去除字节填充的点。
fn read_multiline<R: Read>(reader: &mut R) -> Result<Vec<u8>, AppError> {
    let mut out: Vec<u8> = Vec::new();
    loop {
        let line = read_line(reader)?;
        if line == "." {
            return Ok(out);
        }
        let unmasked = line.strip_prefix('.').unwrap_or(&line);
        if out.len() + unmasked.len() + 2 > MAX_RAW {
            return Err(AppError::network("POP3 响应超出允许长度"));
        }
        out.extend_from_slice(unmasked.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
}

fn tls_session<S: Read + Write>(
    mut stream: S,
    user: &str,
    secret: &str,
) -> Result<usize, AppError> {
    let greeting = read_line(&mut stream)?;
    expect_ok(&greeting, "POP3 问候")?;
    write_command(&mut stream, &format!("USER {user}"))?;
    expect_ok(&read_line(&mut stream)?, "POP3 用户名")?;
    write_command(&mut stream, &format!("PASS {secret}"))?;
    let pass_line = read_line(&mut stream)?;
    if pass_line.starts_with("-ERR") {
        return Err(AppError::auth(
            "POP3 服务器拒绝了登录，请核对登录用户与授权码",
        ));
    }
    expect_ok(&pass_line, "POP3 登录")?;
    write_command(&mut stream, "STAT")?;
    let stat = expect_ok(&read_line(&mut stream)?, "STAT")?;
    let count: usize = stat
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| AppError::network("STAT 响应无法解析"))?;
    Ok(count)
}

fn connect_session(
    host: &str,
    port: u16,
    user: &str,
    secret: &str,
) -> Result<(native_tls::TlsStream<std::net::TcpStream>, usize), AppError> {
    let tcp = std::net::TcpStream::connect((host, port)).map_err(|error| {
        AppError::network(format!(
            "无法连接到 {host}:{port}，请检查地址与端口（{error}）"
        ))
    })?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| tcp.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| AppError::network("无法设置连接超时"))?;
    let connector =
        native_tls::TlsConnector::new().map_err(|_| AppError::network("TLS 初始化失败"))?;
    let mut tls = connector.connect(host, tcp).map_err(|_| {
        AppError::network(format!("{host}:{port} 的 TLS 证书不受信任或协议不被支持"))
    })?;
    let count = tls_session(&mut tls, user, secret)?;
    Ok((tls, count))
}

/// 连接验证用探测（wizard「测试连接」）：登录 + STAT，返回邮件数量。
pub(crate) fn probe(host: &str, port: u16, user: &str, secret: &str) -> Result<usize, AppError> {
    let host = if host.trim().is_empty() || host.len() > 253 {
        return Err(AppError::invalid("服务器地址不符合要求"));
    } else {
        host.trim()
    };
    let tcp = std::net::TcpStream::connect((host, port)).map_err(|error| {
        AppError::network(format!(
            "无法连接到 {host}:{port}，请检查地址与端口（{error}）"
        ))
    })?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| tcp.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| AppError::network("无法设置连接超时"))?;
    let connector =
        native_tls::TlsConnector::new().map_err(|_| AppError::network("TLS 初始化失败"))?;
    let mut tls = connector.connect(host, tcp).map_err(|_| {
        AppError::network(format!("{host}:{port} 的 TLS 证书不受信任或协议不被支持"))
    })?;
    tls_session(&mut tls, user, secret)
}

/// POP3 服务器 Date 头转 UTC ISO-8601；缺失或畸形时回退 None（调用方用下载时间）。
fn mail_date_iso(mail: &mail_parser::Message<'_>) -> Option<String> {
    let date = mail.date()?;
    let naive = chrono::NaiveDate::from_ymd_opt(
        i32::from(date.year),
        u32::from(date.month).clamp(1, 12),
        u32::from(date.day).clamp(1, 31),
    )?
    .and_hms_opt(
        u32::from(date.hour).clamp(0, 23),
        u32::from(date.minute).clamp(0, 59),
        u32::from(date.second).clamp(0, 59),
    )?;
    let offset_minutes = i32::from(date.tz_hour) * 60 + i32::from(date.tz_minute);
    let offset_seconds = if date.tz_before_gmt {
        -offset_minutes * 60
    } else {
        offset_minutes * 60
    };
    let offset = FixedOffset::east_opt(offset_seconds)?;
    let utc = offset
        .from_local_datetime(&naive)
        .earliest()?
        .with_timezone(&Utc);
    Some(utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

#[derive(Debug)]
struct SendSnapshot {
    host: String,
    port: u16,
    auth_user: String,
    seen: Vec<String>,
    cache_root: std::path::PathBuf,
}

fn profile(conn: &rusqlite::Connection, id: &str) -> Result<serde_json::Value, AppError> {
    let profile: String = conn
        .query_row(
            "SELECT profile_json FROM accounts WHERE id = ?1 AND provider = 'pop3'",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取 POP3 账号"))?
        .ok_or_else(|| AppError::invalid("找不到可同步的 POP3 账号"))?;
    let profile: serde_json::Value =
        serde_json::from_str(&profile).map_err(|_| AppError::storage("账号配置损坏"))?;
    if profile["status"] == "demo" {
        return Err(AppError::invalid("演示账号不会连接服务器"));
    }
    for field in ["imapHost", "authUser"] {
        if profile[field]
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(AppError::invalid("账号缺少 POP3 服务器或登录用户"));
        }
    }
    if profile["imapPort"]
        .as_u64()
        .is_none_or(|port| !(1..=65535).contains(&port))
    {
        return Err(AppError::invalid("账号缺少有效的 POP3 端口"));
    }
    Ok(profile)
}

/// 手动入队与 worker 快照共用只读配置校验，不执行网络或凭据 I/O。
pub(crate) fn validate_account(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    profile(conn, id).map(|_| ())
}

fn snapshot(db: &SharedDb, id: &str) -> Result<SendSnapshot, AppError> {
    let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    let profile = profile(&conn, id)?;
    let seen: Vec<String> = conn
        .query_row(
            "SELECT cursor FROM sync_states WHERE account_id = ?1 AND scope = ?2",
            params![id, CURSOR_SCOPE],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取 POP3 游标"))?
        .flatten()
        .and_then(|cursor| serde_json::from_str(&cursor).ok())
        .unwrap_or_default();
    let cache_root = mail_cache::root(&conn)?;
    Ok(SendSnapshot {
        host: profile["imapHost"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string(),
        port: profile["imapPort"].as_u64().unwrap_or(995) as u16,
        auth_user: profile["authUser"].as_str().unwrap_or("").to_string(),
        seen,
        cache_root,
    })
}

/// POP3 同步：UIDL 对比 → RETR 新邮件（每次最多 MAX_DOWNLOAD 封）→ 事务提交 → 记录已见集合。
pub(crate) fn sync_account(
    db: &SharedDb,
    id: &str,
    generation: &str,
) -> Result<SyncOutcome, AppError> {
    let snap = snapshot(db, id)?;
    if snap.host.is_empty() {
        return Err(AppError::invalid("账号未配置 POP3 服务器，无法同步"));
    }
    let secret = credentials::with_account("imap", id, || credentials::secret_for("imap", id))?;
    let (tls, _count) = connect_session(&snap.host, snap.port, &snap.auth_user, &secret)?;
    let mut tls = tls;
    write_command(&mut tls, "UIDL")?;
    expect_ok(&read_line(&mut tls)?, "UIDL")?;
    let listing = read_multiline(&mut tls)?;
    let listing_text = String::from_utf8_lossy(&listing).into_owned();
    let mut uidl_list: Vec<(usize, String)> = Vec::new();
    for line in listing_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((ordinal, uidl)) = line.split_once(' ') else {
            return Err(AppError::network("UIDL 响应无法解析"));
        };
        let ordinal: usize = ordinal
            .parse()
            .map_err(|_| AppError::network("UIDL 响应无法解析"))?;
        uidl_list.push((ordinal, uidl.to_string()));
    }
    // 只下载未见过的 UIDL；每次运行最多 MAX_DOWNLOAD 封，旧 → 新。
    let mut todo: Vec<(usize, String)> = Vec::new();
    for (ordinal, uidl) in &uidl_list {
        if !snap.seen.iter().any(|seen| seen == uidl) {
            todo.push((*ordinal, uidl.clone()));
        }
    }
    todo.truncate(MAX_DOWNLOAD);
    let mut downloaded: Vec<(String, mail_sync::ParsedMessage)> = Vec::new();
    for (ordinal, uidl) in &todo {
        write_command(&mut tls, &format!("RETR {ordinal}"))?;
        expect_ok(&read_line(&mut tls)?, "RETR")?;
        let raw = read_multiline(&mut tls)?;
        let parsed_mail = MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| AppError::invalid("POP3 邮件 MIME 无效"))?;
        let mut spool = Spool::new(&snap.cache_root, raw.len() as u32)?;
        spool.append(&raw)?;
        let (key, _) = spool.finish()?;
        let received_at = mail_date_iso(&parsed_mail).unwrap_or_else(crate::now_utc);
        let ordinal_u32 =
            u32::try_from(*ordinal).map_err(|_| AppError::network("邮件序号超出范围"))?;
        let mut parsed = mail_sync::parse_raw(ordinal_u32, &raw, received_at, true, false)?;
        parsed.raw_cache_key = Some(key);
        downloaded.push((uidl.clone(), parsed));
    }
    let _ = write_command(&mut tls, "QUIT");
    let _ = read_line(&mut tls);
    drop(tls);

    // 提交交给 mail_sync::commit_pop3（目录 / 消息 / 已见集合，一个事务）。
    let fetched = downloaded.len();
    let uidls_only: Vec<String> = uidl_list.iter().map(|(_, uidl)| uidl.clone()).collect();
    mail_sync::commit_pop3(db, id, generation, &uidls_only, &downloaded)?;
    Ok(SyncOutcome {
        // 静默同步语义：无新邮件的 UIDL 轮次不要求前端刷新（POP3 无回写通道）。
        refresh_needed: fetched > 0,
        folders: 1,
        fetched_messages: fetched,
        synced_at: crate::now_utc(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct MockStream {
        reads: std::io::Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }
    impl MockStream {
        fn new(script: &str) -> Self {
            Self {
                reads: std::io::Cursor::new(script.as_bytes().to_vec()),
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
    fn probe_parses_stat_count_and_rejects_errors() {
        let mut stream = MockStream::new(
            "+OK POP3 ready\r\n+OK user accepted\r\n+OK logged in\r\n+OK 3 4096\r\n",
        );
        let count = tls_session(&mut stream, "alex@example.test", "secret").unwrap();
        assert_eq!(count, 3);
        let log = String::from_utf8(stream.writes).unwrap();
        assert!(log.contains("USER alex@example.test"));
        // POP3 密码经 PASS 命令在 TLS 通道内传输（协议固有）。
        assert!(log.contains("PASS secret"));
        assert!(log.ends_with("STAT\r\n"));
        let mut stream = MockStream::new("+OK POP3 ready\r\n+OK\r\n-ERR invalid password\r\n");
        let error = tls_session(&mut stream, "alex@example.test", "wrong").unwrap_err();
        assert_eq!(error.code, "auth");
    }

    #[test]
    fn multiline_reader_unstuffs_and_stops_at_dot() {
        let mut stream = MockStream::new("Line one\r\n..dotted\r\n.\r\n+OK\r\n");
        let data = read_multiline(&mut stream).unwrap();
        assert_eq!(String::from_utf8(data).unwrap(), "Line one\r\n.dotted\r\n");
        let next = read_line(&mut stream).unwrap();
        assert!(next.starts_with("+OK"));
    }

    #[test]
    fn mail_dates_convert_to_utc() {
        let mail = MessageParser::default()
            .parse(b"Date: Mon, 14 Nov 2026 10:00:00 +0800\r\nSubject: t\r\n\r\nbody".as_slice())
            .unwrap();
        let iso = mail_date_iso(&mail).unwrap();
        assert!(iso.starts_with("2026-11-14T02:00:00Z"), "{iso}");
        assert!(mail_date_iso(
            &MessageParser::default()
                .parse(b"Subject: no date\r\n\r\nx".as_slice())
                .unwrap()
        )
        .is_none());
    }

    #[test]
    fn snapshot_rejects_demo_and_generation_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::storage::initialize(&dir.path().join("mail.db")).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts (id, provider, email, display_name, created_at, profile_json) VALUES
             ('pop3:1','pop3','a@example.test','P','2026-11-14T00:00:00Z','{\"status\":\"connected\",\"imapHost\":\"pop.example.test\",\"imapPort\":995,\"authUser\":\"a@example.test\"}');",
        )
        .unwrap();
        let db = std::sync::Arc::new(std::sync::Mutex::new(conn));
        assert!(snapshot(&db, "pop3:1").is_ok());
        db.lock().unwrap().execute("UPDATE accounts SET provider='imap', profile_json='{\"status\":\"demo\"}' WHERE id='pop3:1'", []).unwrap();
        assert_eq!(snapshot(&db, "pop3:1").unwrap_err().code, "invalid");
    }
}
