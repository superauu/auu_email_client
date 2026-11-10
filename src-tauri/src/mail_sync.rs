//! Incremental IMAP sync for real accounts.
//! Cursors live in `sync_states` as {uidValidity, lastUid} per folder scope. A changed
//! UIDVALIDITY purges that folder's stale rows and re-windows the newest messages.
use crate::error::AppError;
use crate::mail_verify;
use mail_parser::MessageParser;
use rusqlite::{params, Connection};
use serde::Serialize;

const INITIAL_WINDOW: u32 = 50;
const SYNCED_KINDS: [&str; 2] = ["inbox", "sent"];

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub folders: usize,
    pub fetched_messages: usize,
    pub synced_at: String,
    pub warnings: Vec<String>,
}

struct Endpoints {
    host: String,
    port: u16,
    auth_user: String,
}

/// Maps IMAP mailbox names to the five folder kinds the UI understands.
fn folder_kind(name: &str) -> &'static str {
    let normalized = name.to_lowercase();
    if normalized == "inbox" {
        "inbox"
    } else if normalized.contains("sent") || normalized.contains("已发送") {
        "sent"
    } else if normalized.contains("draft") || normalized.contains("草稿") {
        "drafts"
    } else if normalized.contains("archive") || normalized.contains("归档") {
        "archive"
    } else if normalized.contains("trash")
        || normalized.contains("deleted")
        || normalized.contains("废纸")
    {
        "trash"
    } else {
        "other"
    }
}

fn load_account(conn: &Connection, account_id: &str) -> Result<serde_json::Value, AppError> {
    let profile: Option<String> = conn
        .query_row(
            "SELECT profile_json FROM accounts WHERE id = ?1 AND provider = 'imap'",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::invalid("找不到可同步的账号"))?;
    profile
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| AppError::invalid("账号缺少服务器信息"))
}

fn endpoints_of(profile: &serde_json::Value) -> Result<Endpoints, AppError> {
    let field = |key: &str| {
        profile
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::to_string)
    };
    let host = field("imapHost").ok_or_else(|| AppError::invalid("账号缺少 IMAP 服务器信息"))?;
    let auth_user = field("authUser").ok_or_else(|| AppError::invalid("账号缺少登录用户"))?;
    let port = profile
        .get("imapPort")
        .and_then(|value| value.as_u64())
        .filter(|port| (1..=65535).contains(port))
        .ok_or_else(|| AppError::invalid("账号缺少有效的 IMAP 端口"))? as u16;
    Ok(Endpoints {
        host,
        port,
        auth_user,
    })
}

fn login_failure(error: imap::Error) -> AppError {
    match error {
        imap::Error::No(_) | imap::Error::Bad(_) => {
            AppError::auth("IMAP 服务器拒绝了登录，请核对账号密码")
        }
        _ => AppError::network("IMAP 会话未能建立"),
    }
}

/// Syncs INBOX and Sent for one real account; folder discovery runs first.
pub fn sync_account(
    conn: &mut Connection,
    account_id: &str,
    secret: &str,
) -> Result<SyncOutcome, AppError> {
    let profile = load_account(conn, account_id)?;
    let endpoints = endpoints_of(&profile)?;
    let tls = mail_verify::tls_connect(&endpoints.host, endpoints.port)?;
    let client = imap::Client::new(tls);
    let session = client
        .login(&endpoints.auth_user, secret)
        .map_err(|(error, _)| login_failure(error))?;
    sync_account_with_session(conn, account_id, session)
}

fn sync_account_with_session<S: std::io::Read + std::io::Write>(
    conn: &mut Connection,
    account_id: &str,
    mut session: imap::Session<S>,
) -> Result<SyncOutcome, AppError> {
    let names: Vec<String> = session
        .list(Some(""), Some("*"))
        .map_err(|_| AppError::network("无法读取邮箱文件夹列表"))?
        .iter()
        .map(|name| name.name().to_string())
        .collect();
    upsert_folders(conn, account_id, &names)?;

    let mut outcome = SyncOutcome {
        folders: names.len(),
        synced_at: crate::now_utc(),
        ..SyncOutcome::default()
    };
    for kind in SYNCED_KINDS {
        let Some(remote) = names.iter().find(|name| folder_kind(name) == kind) else {
            continue;
        };
        match sync_folder(conn, &mut session, account_id, kind, remote) {
            Ok(fetched) => outcome.fetched_messages += fetched,
            Err(error) if kind == "sent" => outcome
                .warnings
                .push(format!("Sent 文件夹同步失败：{}", error.message)),
            Err(error) => {
                let _ = session.logout();
                return Err(error);
            }
        }
    }
    let _ = session.logout();
    Ok(outcome)
}

fn upsert_folders(
    conn: &mut Connection,
    account_id: &str,
    names: &[String],
) -> Result<(), AppError> {
    let transaction = conn
        .transaction()
        .map_err(|e| AppError::storage(e.to_string()))?;
    for name in names {
        let folder_id = format!("{account_id}:folder:{name}");
        transaction
            .execute(
                "INSERT INTO folders (id, account_id, remote_id, name, kind) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET name = excluded.name, kind = excluded.kind",
                params![folder_id, account_id, name, name, folder_kind(name)],
            )
            .map_err(|e| AppError::storage(e.to_string()))?;
    }
    transaction
        .commit()
        .map_err(|e| AppError::storage(e.to_string()))
}

fn read_cursor(conn: &Connection, account_id: &str, scope: &str) -> Option<(u32, u32)> {
    let raw: String = conn
        .query_row(
            "SELECT cursor FROM sync_states WHERE account_id = ?1 AND scope = ?2",
            params![account_id, scope],
            |row| row.get(0),
        )
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some((
        value.get("uidValidity")?.as_u64()? as u32,
        value.get("lastUid")?.as_u64()? as u32,
    ))
}

fn write_cursor(
    conn: &mut Connection,
    account_id: &str,
    scope: &str,
    uid_validity: u32,
    last_uid: u32,
) -> Result<(), AppError> {
    let cursor =
        serde_json::json!({ "uidValidity": uid_validity, "lastUid": last_uid }).to_string();
    conn.execute(
        "INSERT INTO sync_states (account_id, scope, cursor, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, scope) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at",
        params![account_id, scope, cursor, crate::now_utc()],
    )
    .map_err(|e| AppError::storage(e.to_string()))?;
    Ok(())
}

/// Drops rows from an older UIDVALIDITY so stale UIDs cannot resurface as duplicates.
fn purge_stale(
    conn: &mut Connection,
    account_id: &str,
    kind: &str,
    uid_validity: u32,
) -> Result<(), AppError> {
    let stale_pattern = format!("imap:{kind}:%");
    let fresh_pattern = format!("imap:{kind}:{uid_validity}:%");
    conn.execute(
        "DELETE FROM messages WHERE account_id = ?1 AND remote_id LIKE ?2 AND remote_id NOT LIKE ?3",
        params![account_id, stale_pattern, fresh_pattern],
    )
    .map_err(|e| AppError::storage(e.to_string()))?;
    Ok(())
}

fn sync_folder<S: std::io::Read + std::io::Write>(
    conn: &mut Connection,
    session: &mut imap::Session<S>,
    account_id: &str,
    kind: &str,
    remote_name: &str,
) -> Result<usize, AppError> {
    let mailbox = session
        .select(remote_name)
        .map_err(|_| AppError::network(format!("无法打开 {remote_name} 文件夹")))?;
    let uid_validity = mailbox
        .uid_validity
        .ok_or_else(|| AppError::network("IMAP 服务器未提供 UIDVALIDITY"))?;
    let uid_next = mailbox.uid_next.unwrap_or(1);
    let scope = format!("imap:{kind}");
    let cursor = read_cursor(conn, account_id, &scope);
    let (start, incremental) = match cursor {
        Some((cursor_validity, last_uid)) if cursor_validity == uid_validity => {
            (last_uid.saturating_add(1).max(1), true)
        }
        _ => (uid_next.saturating_sub(INITIAL_WINDOW).max(1), false),
    };
    if !incremental {
        purge_stale(conn, account_id, kind, uid_validity)?;
    }
    let mut fetched = 0usize;
    let mut highest_uid = start.saturating_sub(1);
    if uid_next > 1 && start < uid_next {
        let batch = session
            .uid_fetch(format!("{start}:*"), "(UID FLAGS INTERNALDATE BODY.PEEK[])")
            .map_err(|_| AppError::network(format!("同步 {remote_name} 时读取失败")))?;
        let folder_id = format!("{account_id}:folder:{remote_name}");
        let transaction = conn
            .transaction()
            .map_err(|e| AppError::storage(e.to_string()))?;
        for fetch in batch.iter() {
            let Some(uid) = fetch.uid else { continue };
            highest_uid = highest_uid.max(uid);
            let Some(raw) = fetch.body() else { continue };
            let seen = fetch
                .flags()
                .iter()
                .any(|flag| matches!(flag, imap::types::Flag::Seen));
            let flagged = fetch
                .flags()
                .iter()
                .any(|flag| matches!(flag, imap::types::Flag::Flagged));
            let received_at = fetch
                .internal_date()
                .map(|date| date.naive_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_else(crate::now_utc);
            apply_message(
                &transaction,
                account_id,
                kind,
                uid_validity,
                uid,
                raw,
                seen,
                flagged,
                &received_at,
                &folder_id,
            )?;
            fetched += 1;
        }
        transaction
            .commit()
            .map_err(|e| AppError::storage(e.to_string()))?;
    }
    write_cursor(conn, account_id, &scope, uid_validity, highest_uid)?;
    Ok(fetched)
}

#[allow(clippy::too_many_arguments)]
fn apply_message(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
    kind: &str,
    uid_validity: u32,
    uid: u32,
    raw: &[u8],
    seen: bool,
    flagged: bool,
    received_at: &str,
    folder_id: &str,
) -> Result<(), AppError> {
    let parsed = MessageParser::default().parse(raw);
    let subject = parsed
        .as_ref()
        .and_then(|message| message.subject())
        .unwrap_or("（无主题）")
        .to_string();
    let from = parsed
        .as_ref()
        .and_then(|message| message.from())
        .and_then(|addresses| addresses.first())
        .map(|address| {
            let name = address.name().unwrap_or("").to_string();
            let email = address.address().unwrap_or("").to_string();
            (if name.is_empty() { email.clone() } else { name }, email)
        })
        .unwrap_or_else(|| ("未知发件人".to_string(), String::new()));
    let to: Vec<String> = parsed
        .as_ref()
        .and_then(|message| message.to())
        .map(|addresses| {
            addresses
                .iter()
                .filter_map(|address| address.address().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let body_text: String = parsed
        .as_ref()
        .and_then(|message| message.body_text(0))
        .map(|text| text.to_string())
        .unwrap_or_default();
    let message_id = format!("{account_id}:imap:{kind}:{uid_validity}:{uid}");
    let remote_id = format!("imap:{kind}:{uid_validity}:{uid}");

    transaction
        .execute(
            "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, sender_name, sender_email, recipients_json, body_text, received_at, unread, starred)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(account_id, remote_id) DO UPDATE SET
               subject = excluded.subject, sender_name = excluded.sender_name, sender_email = excluded.sender_email,
               recipients_json = excluded.recipients_json, body_text = excluded.body_text,
               received_at = excluded.received_at, unread = excluded.unread, starred = excluded.starred",
            params![
                message_id, account_id, remote_id, message_id, subject, from.0, from.1,
                serde_json::to_string(&to).map_err(|e| AppError::storage(e.to_string()))?,
                body_text, received_at, !seen, flagged,
            ],
        )
        .map_err(|e| AppError::storage(e.to_string()))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO message_folders (account_id, message_id, folder_id) VALUES (?1, ?2, ?3)",
            params![account_id, message_id, folder_id],
        )
        .map_err(|e| AppError::storage(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    use std::io::{Cursor, Read, Write};

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

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        storage::migrate(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, provider, email, display_name, profile_json, created_at)
             VALUES ('acc', 'imap', 'me@example.test', '测试账号', ?1, '2026-01-01T00:00:00Z')",
            params![serde_json::json!({
                "imapHost": "127.0.0.1", "imapPort": 1, "smtpHost": "127.0.0.1", "smtpPort": 1,
                "authUser": "me@example.test", "avatar": "测", "color": "teal", "status": "connected",
                "capabilities": {}
            }).to_string()],
        ).unwrap();
        conn
    }

    fn session_script(fetch_body: &str, uid: u32, uid_next: u32, uid_validity: u32) -> String {
        let literal_len = fetch_body.len();
        format!(
            "* OK IMAP4rev1 ready\r\n\
             a1 OK Logged in\r\n\
             * LIST (\\HasNoChildren) \"/\" INBOX\r\n\
             a2 OK LIST completed\r\n\
             * 1 EXISTS\r\n\
             * OK [UIDVALIDITY {uid_validity}] UIDs valid\r\n\
             * OK [UIDNEXT {uid_next}] Predicted next UID\r\n\
             a3 OK [READ-WRITE] SELECT completed\r\n\
             * 1 FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"10-Nov-2026 03:24:00 +0000\" BODY[] {{{literal_len}}}\r\n{fetch_body})\r\n\
             a4 OK FETCH completed\r\n\
             * BYE Logging out\r\n\
             a5 OK Logout completed\r\n"
        )
    }

    fn run_session(conn: &mut Connection, script: String) -> SyncOutcome {
        let client = imap::Client::new(MockStream::new(&script));
        let session = client.login("me@example.test", "secret").unwrap();
        sync_account_with_session(conn, "acc", session).unwrap()
    }

    #[test]
    fn first_sync_stores_message_text_flags_and_cursor() {
        let mut conn = database();
        let body = "Subject: =?UTF-8?B?5L2g5aW9?=\r\nFrom: Alice <alice@example.test>\r\nTo: me@example.test\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from Alice\r\n";
        let outcome = run_session(&mut conn, session_script(body, 1001, 1002, 1000));
        assert_eq!(outcome.fetched_messages, 1);
        assert!(outcome.warnings.is_empty());
        let (subject, sender, unread, body_text): (String, String, i64, String) = conn
            .query_row("SELECT subject, sender_name, unread, body_text FROM messages WHERE account_id = 'acc'", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap();
        assert_eq!(subject, "你好"); // RFC 2047 decoded
        assert_eq!(sender, "Alice");
        assert_eq!(unread, 0); // \Seen flag => read
        assert!(body_text.contains("Hello from Alice"));
        let kind: String = conn
            .query_row(
                "SELECT kind FROM folders WHERE account_id = 'acc' AND remote_id = 'INBOX'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "inbox");
        let cursor: String = conn
            .query_row(
                "SELECT cursor FROM sync_states WHERE scope = 'imap:inbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(cursor.contains("\"lastUid\":1001"));
    }

    #[test]
    fn incremental_sync_appends_only_new_uids() {
        let mut conn = database();
        let body = "Subject: First\r\nFrom: Alice <alice@example.test>\r\n\r\nFirst body\r\n";
        run_session(&mut conn, session_script(body, 1001, 1002, 1000));
        // The second session advertises UIDNEXT 1003 and serves one new UID.
        let second = "Subject: Second\r\nFrom: Bob <bob@example.test>\r\n\r\nSecond body\r\n";
        run_session(&mut conn, session_script(second, 1002, 1003, 1000));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = 'acc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        let cursor: String = conn
            .query_row(
                "SELECT cursor FROM sync_states WHERE scope = 'imap:inbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(cursor.contains("\"lastUid\":1002"));
    }

    #[test]
    fn changed_uid_validity_purges_stale_messages() {
        let mut conn = database();
        let body = "Subject: Old\r\nFrom: A <a@example.test>\r\n\r\nOld\r\n";
        run_session(&mut conn, session_script(body, 1001, 1002, 1000));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        // UIDVALIDITY changes to 2000 => the old row is purged before applying the new window.
        let body2 = "Subject: New\r\nFrom: B <b@example.test>\r\n\r\nNew\r\n";
        run_session(&mut conn, session_script(body2, 5001, 5002, 2000));
        let subjects: Vec<String> = conn
            .prepare("SELECT subject FROM messages ORDER BY subject")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(subjects, vec!["New"]);
    }

    #[test]
    fn folder_kind_mapping_covers_common_names() {
        assert_eq!(folder_kind("INBOX"), "inbox");
        assert_eq!(folder_kind("Sent Items"), "sent");
        assert_eq!(folder_kind("已发送"), "sent");
        assert_eq!(folder_kind("Drafts"), "drafts");
        assert_eq!(folder_kind("Archive/2026"), "archive");
        assert_eq!(folder_kind("Deleted Items"), "trash");
        assert_eq!(folder_kind("Projects"), "other");
    }
}
