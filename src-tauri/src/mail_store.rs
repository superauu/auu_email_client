//! SQLite repository for the mail workspace: idempotent demo seeding, paged message queries.
//! Timestamps are UTC ISO-8601 strings; keyset pagination orders by (received_at DESC, id DESC).
use crate::error::AppError;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_PAGE_LIMIT: u32 = 500;
const DEFAULT_PAGE_LIMIT: u32 = 100;
const PREVIEW_CHARS: usize = 120;
/// Folder kinds map 1:1 to the UI mailbox projection; 'starred' is a flag filter, never a folder.
const FOLDER_KINDS: [&str; 6] = ["inbox", "sent", "drafts", "archive", "trash", "other"];
/// Deterministic projection when a message sits in several folders.
const KIND_PRIORITY: &str = "CASE f.kind WHEN 'inbox' THEN 0 WHEN 'sent' THEN 1 WHEN 'drafts' THEN 2 WHEN 'archive' THEN 3 ELSE 4 END";

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SeedWorkspace {
    pub accounts: Vec<SeedAccount>,
    pub labels: Vec<SeedLabel>,
    pub messages: Vec<SeedMessage>,
    pub events: Vec<SeedEvent>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SeedAccount {
    pub id: String,
    pub provider: String,
    pub email: String,
    pub name: String,
    pub avatar: String,
    pub color: String,
    pub status: String,
    pub capabilities: Value,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SeedLabel {
    pub id: String,
    pub name: String,
    pub color: String,
    pub scope: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SeedMessage {
    pub id: String,
    pub account_id: String,
    #[serde(default)]
    pub remote_id: Option<String>,
    pub thread_id: String,
    pub mailbox: String,
    pub from: SeedAddress,
    #[serde(default)]
    pub to: Vec<String>,
    pub subject: String,
    #[serde(default)]
    pub body_text: String,
    pub received_at: String,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub label_ids: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SeedAddress {
    pub name: String,
    pub email: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SeedEvent {
    pub id: String,
    pub account_id: String,
    #[serde(default)]
    pub message_id: Option<String>,
    pub title: String,
    pub starts_at: String,
    pub time_zone: String,
    #[serde(default)]
    pub completed: bool,
    pub source: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBase {
    pub folders: Vec<FolderRow>,
    pub accounts: Vec<AccountRow>,
    pub labels: Vec<LabelRow>,
    pub events: Vec<EventRow>,
    pub sync_states: Vec<SyncStateRow>,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub kind: String,
    pub available: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyncStateRow {
    pub account_id: String,
    pub scope: String,
    pub updated_at: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountRow {
    pub oauth: Option<crate::mail_oauth::Config>,
    pub auth_user: String,
    pub id: String,
    pub name: String,
    pub email: String,
    pub provider: String,
    pub avatar: String,
    pub color: String,
    pub status: String,
    pub capabilities: Value,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LabelRow {
    pub id: String,
    pub account_id: Option<String>,
    pub remote_id: Option<String>,
    pub name: String,
    pub color: String,
    pub scope: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EventRow {
    pub id: String,
    pub account_id: String,
    pub message_id: Option<String>,
    pub title: String,
    pub starts_at: String,
    pub time_zone: String,
    pub completed: bool,
    pub source: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub body_download: Option<crate::imap_body::Status>,
    pub folder_ids: Vec<String>,
    pub attachments: Vec<crate::mail_sync::Attachment>,
    pub remote_missing: bool,
    pub recovery: Option<crate::mail_recovery::Status>,
    #[serde(skip)]
    pub content_hash: Option<String>,
    #[serde(skip)]
    pub locator_verified: bool,
    #[serde(skip)]
    pub content_hash_verified: bool,
    #[serde(skip)]
    pub header_message_id: Option<String>,
    pub state_revision: i64,
    pub changes: Vec<crate::message_changes::ChangeStatus>,
    pub id: String,
    pub account_id: String,
    pub remote_id: Option<String>,
    pub thread_id: String,
    pub mailbox: String,
    pub from: FromRow,
    pub to: Vec<String>,
    pub subject: String,
    pub preview: String,
    pub body_text: String,
    pub received_at: String,
    pub unread: bool,
    pub starred: bool,
    pub label_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<crate::drafts::Draft>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FromRow {
    pub name: String,
    pub email: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub items: Vec<MessageRow>,
    pub next_cursor: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageQuery {
    pub folder_id: Option<String>,
    pub message_id: Option<String>,
    pub account_id: Option<String>,
    /// Folder kind or 'starred' (starred flag excluding trash), matching the UI mailbox projection.
    pub mailbox: Option<String>,
    pub label_id: Option<String>,
    pub search: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

fn validate_folder_kind(kind: &str) -> Result<(), AppError> {
    if FOLDER_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(AppError::invalid(format!("未知的邮件文件夹类型: {kind}")))
    }
}

fn preview_of(body: Option<&str>) -> String {
    body.unwrap_or("").chars().take(PREVIEW_CHARS).collect()
}

/// FTS5 phrase-quotes every whitespace term so caller input can never inject MATCH syntax.
fn fts_expression(search: &str) -> String {
    search
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn cursor_encode(received_at: &str, id: &str) -> String {
    format!("{received_at}\u{1f}{id}")
}

fn cursor_decode(cursor: &str) -> Result<(String, String), AppError> {
    let (received_at, id) = cursor
        .split_once('\u{1f}')
        .ok_or_else(|| AppError::invalid("无效的分页游标"))?;
    if received_at.is_empty() || id.is_empty() {
        return Err(AppError::invalid("无效的分页游标"));
    }
    Ok((received_at.to_string(), id.to_string()))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RealAccount {
    #[serde(default)]
    pub receive_only: bool,
    #[serde(default)]
    pub oauth: Option<crate::mail_oauth::Config>,
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    #[serde(default)]
    pub imap_security: crate::imap_transport::Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub auth_user: String,
    pub avatar: String,
    pub color: String,
}

/// Persists a wizard-verified real IMAP account. Caller owns the connection probe and the
/// keyring write; if the DB insert fails afterwards the caller compensates by deleting the entry.
pub fn save_real_account(
    conn: &mut Connection,
    account: &RealAccount,
    expected: Option<&str>,
) -> Result<String, AppError> {
    if !account.email.contains('@')
        || account.name.trim().is_empty()
        || account.auth_user.trim().is_empty()
    {
        return Err(AppError::invalid("账号信息不完整"));
    }
    let id = account
        .id
        .clone()
        .unwrap_or_else(|| format!("imap:{}", uuid::Uuid::new_v4()));
    let profile = serde_json::json!({
        "avatar": account.avatar,
        "color": account.color,
        "status": "connected",
        "capabilities": { "folders": true, "serverLabels": false, "readStateSync": true, "threads": true, "calendar": false },
        "imapHost": account.imap_host,
        "imapPort": account.imap_port,
        "imapSecurity": account.imap_security,
        "smtpHost": account.smtp_host,
        "smtpPort": account.smtp_port,
        "authUser": account.auth_user,
        "receiveOnly":account.receive_only,
        "oauth":account.oauth,
        "authEpoch":uuid::Uuid::new_v4().to_string(),
    });
    let transaction = conn
        .transaction()
        .map_err(|e| AppError::storage(e.to_string()))?;
    let current: Option<String> = transaction
        .query_row(
            "SELECT sync_generation FROM accounts WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法核对账号版本"))?;
    if current.as_deref() != expected {
        return Err(AppError::conflict("账号版本已改变，未覆盖连接或授权"));
    }
    transaction
        .execute(
            "INSERT INTO accounts (id, provider, email, display_name, profile_json, created_at) VALUES (?1, 'imap', ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET email = excluded.email, display_name = excluded.display_name, profile_json = excluded.profile_json",
            params![id, account.email, account.name.trim(), profile.to_string(), crate::now_utc()],
        )
        .map_err(|e| AppError::storage(e.to_string()))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO imap_settings(account_id) VALUES (?1)",
            [&id],
        )
        .map_err(|_| AppError::storage("无法保存 IMAP 设置"))?;
    transaction
        .execute(
            "DELETE FROM sync_states WHERE account_id=?1 AND scope LIKE 'imap:%'",
            [&id],
        )
        .map_err(|_| AppError::storage("无法重建账号同步边界"))?;
    transaction
        .execute("DELETE FROM imap_sync_requests WHERE account_id=?1", [&id])
        .map_err(|_| AppError::storage("无法清理旧连接请求"))?;
    transaction.execute("UPDATE messages SET remote_missing=1,state_revision=state_revision+1 WHERE account_id=?1 AND remote_missing=0",[&id]).map_err(|_|AppError::storage("无法保护旧连接邮件"))?;
    transaction.execute("UPDATE message_changes SET state='conflict',error='连接已重新配置，保留本机修改，需明确核对定位。' WHERE account_id=?1 AND state IN ('queued','failed')",[&id]).map_err(|_|AppError::storage("无法保留旧连接修改"))?;
    transaction
        .commit()
        .map_err(|e| AppError::storage(e.to_string()))?;
    Ok(id)
}

/// Removes a real account; folders and messages cascade with it, local events detach.
pub fn delete_real_account(conn: &mut Connection, id: &str) -> Result<(), AppError> {
    let transaction = conn
        .transaction()
        .map_err(|e| AppError::storage(e.to_string()))?;
    transaction
        .execute("DELETE FROM accounts WHERE id = ?1", params![id])
        .map_err(|e| AppError::storage(e.to_string()))?;
    transaction
        .commit()
        .map_err(|e| AppError::storage(e.to_string()))
}

/// Refreshes demo messages without deleting accounts or their durable drafts.
pub fn seed_workspace(conn: &mut Connection, seed: &SeedWorkspace) -> Result<(), AppError> {
    if seed.accounts.iter().any(|account| account.status != "demo") {
        return Err(AppError::invalid("只允许初始化演示账号"));
    }
    for message in &seed.messages {
        validate_folder_kind(&message.mailbox)?;
    }
    for label in &seed.labels {
        if label.scope != "local" {
            return Err(AppError::invalid("演示标签只能是本地作用域"));
        }
    }
    let transaction = conn
        .transaction()
        .map_err(|e| AppError::storage(e.to_string()))?;
    {
        // Never delete account rows here: drafts must survive every workspace reload.
        for label in &seed.labels {
            transaction
                .execute("DELETE FROM labels WHERE id = ?1", params![label.id])
                .map_err(|e| AppError::storage(e.to_string()))?;
        }
        for account in &seed.accounts {
            let profile = serde_json::json!({ "avatar": account.avatar, "color": account.color, "status": account.status, "capabilities": account.capabilities });
            transaction.execute(
                "INSERT INTO accounts (id, provider, email, display_name, profile_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET provider = excluded.provider, email = excluded.email,
                   display_name = excluded.display_name, profile_json = excluded.profile_json",
                params![account.id, account.provider, account.email, account.name, profile.to_string(), crate::now_utc()],
            ).map_err(|e| AppError::storage(e.to_string()))?;
        }
        for label in &seed.labels {
            transaction.execute(
                "INSERT INTO labels (id, account_id, remote_id, name, color, scope) VALUES (?1, NULL, NULL, ?2, ?3, ?4)",
                params![label.id, label.name, label.color, label.scope],
            ).map_err(|e| AppError::storage(e.to_string()))?;
        }
        for message in &seed.messages {
            let remote_id = message.remote_id.clone().unwrap_or_else(|| {
                format!(
                    "demo:{}:{}:{}",
                    message.account_id, message.mailbox, message.id
                )
            });
            let folder_id = format!("{}:{}", message.account_id, message.mailbox);
            transaction.execute(
                "INSERT OR IGNORE INTO folders (id, account_id, remote_id, name, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![folder_id, message.account_id, message.mailbox, message.mailbox, message.mailbox],
            ).map_err(|e| AppError::storage(e.to_string()))?;
            transaction.execute(
                "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, sender_name, sender_email, recipients_json, body_text, received_at, unread, starred)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   thread_id = excluded.thread_id, subject = excluded.subject, sender_name = excluded.sender_name,
                   sender_email = excluded.sender_email, recipients_json = excluded.recipients_json, body_text = excluded.body_text,
                   received_at = excluded.received_at,
                   unread = COALESCE((SELECT value FROM message_changes WHERE message_id=messages.id AND field='unread'),excluded.unread),
                   starred = COALESCE((SELECT value FROM message_changes WHERE message_id=messages.id AND field='starred'),excluded.starred)",
                params![
                    message.id, message.account_id, remote_id, message.thread_id, message.subject, message.from.name,
                    message.from.email, serde_json::to_string(&message.to).map_err(|e| AppError::storage(e.to_string()))?,
                    message.body_text, message.received_at, message.unread, message.starred,
                ],
            ).map_err(|e| AppError::storage(e.to_string()))?;
            transaction.execute(
                "INSERT OR IGNORE INTO message_folders (account_id, message_id, folder_id) VALUES (?1, ?2, ?3)",
                params![message.account_id, message.id, folder_id],
            ).map_err(|e| AppError::storage(e.to_string()))?;
            for label_id in &message.label_ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
                    params![message.id, label_id],
                ).map_err(|e| AppError::storage(e.to_string()))?;
            }
        }
        for event in &seed.events {
            transaction.execute(
                "INSERT INTO mail_events (id, account_id, message_id, title, starts_at, time_zone, completed, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET message_id = excluded.message_id, title = excluded.title, starts_at = excluded.starts_at,
                   time_zone = excluded.time_zone, completed = excluded.completed, source = excluded.source",
                params![event.id, event.account_id, event.message_id, event.title, event.starts_at, event.time_zone, event.completed, event.source],
            ).map_err(|e| AppError::storage(e.to_string()))?;
        }
    }
    transaction
        .commit()
        .map_err(|e| AppError::storage(e.to_string()))
}

fn query_rows<T, F>(conn: &Connection, sql: &str, map: F) -> Result<Vec<T>, AppError>
where
    F: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::storage(e.to_string()))?;
    let rows = stmt
        .query_map([], map)
        .map_err(|e| AppError::storage(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::storage(e.to_string()))
}

pub fn load_workspace(conn: &Connection) -> Result<WorkspaceBase, AppError> {
    let folders=query_rows(conn,"SELECT f.id,f.account_id,f.name,f.kind,COALESCE(s.available,1) FROM folders f LEFT JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id ORDER BY f.account_id,f.name",|r|Ok(FolderRow{id:r.get(0)?,account_id:r.get(1)?,name:r.get(2)?,kind:r.get(3)?,available:r.get(4)?}))?;
    let accounts = query_rows(
        conn,
        "SELECT id, display_name, email, provider, profile_json FROM accounts ORDER BY created_at, id",
        |row| {
            let profile: Option<String> = row.get(4)?;
            let profile: Value = profile
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_else(default_profile);
            Ok(AccountRow {
                oauth:profile.get("oauth").filter(|v|!v.is_null()).and_then(|v|serde_json::from_value(v.clone()).ok()),
                auth_user:profile["authUser"].as_str().unwrap_or("").to_owned(),
                id: row.get(0)?,
                name: row.get(1)?,
                email: row.get(2)?,
                provider: row.get(3)?,
                avatar: profile["avatar"].as_str().unwrap_or("·").to_string(),
                color: profile["color"].as_str().unwrap_or("teal").to_string(),
                status: profile["status"].as_str().unwrap_or("offline").to_string(),
                capabilities: if row.get::<_,String>(3)?=="imap" && profile["status"]!="demo" {
                    let mut capabilities=profile["capabilities"].as_object().cloned().unwrap_or_default();capabilities.insert("readStateSync".into(),Value::Bool(true));Value::Object(capabilities)
                } else {profile["capabilities"].clone()},
            })
        },
    )?;

    let labels = query_rows(
        conn,
        "SELECT id, account_id, remote_id, name, color, scope FROM labels ORDER BY name, id",
        |row| {
            Ok(LabelRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                remote_id: row.get(2)?,
                name: row.get(3)?,
                color: row.get(4)?,
                scope: row.get(5)?,
            })
        },
    )?;

    let events = query_rows(
        conn,
        "SELECT id, account_id, message_id, title, starts_at, time_zone, completed, source FROM mail_events ORDER BY starts_at, id",
        |row| {
            Ok(EventRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                message_id: row.get(2)?,
                title: row.get(3)?,
                starts_at: row.get(4)?,
                time_zone: row.get(5)?,
                completed: row.get::<_, i64>(6)? != 0,
                source: row.get(7)?,
            })
        },
    )?;

    let sync_states = query_rows(
        conn,
        "SELECT account_id, scope, updated_at FROM sync_states ORDER BY account_id, scope",
        |row| {
            Ok(SyncStateRow {
                account_id: row.get(0)?,
                scope: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    )?;

    Ok(WorkspaceBase {
        folders,
        accounts,
        labels,
        events,
        sync_states,
    })
}

fn default_profile() -> Value {
    serde_json::json!({ "avatar": "·", "color": "teal", "status": "offline", "capabilities": { "folders": false, "serverLabels": false, "readStateSync": false, "threads": false, "calendar": false } })
}

pub fn page_messages(conn: &Connection, query: &MessageQuery) -> Result<MessagePage, AppError> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .clamp(1, MAX_PAGE_LIMIT);
    if let Some(mailbox) = query.mailbox.as_deref() {
        if mailbox != "starred" {
            validate_folder_kind(mailbox)?;
        }
    }
    let mut clauses: Vec<&'static str> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(id) = &query.message_id {
        clauses.push("m.id = ?");
        args.push(Box::new(id.clone()));
    }
    if let Some(account) = &query.account_id {
        clauses.push("m.account_id = ?");
        args.push(Box::new(account.clone()));
    }
    let starred_only = query.mailbox.as_deref() == Some("starred");
    let folder_kind = if starred_only {
        None
    } else {
        query.mailbox.clone()
    };
    if let Some(folder) = &query.folder_id {
        clauses.push("EXISTS(SELECT 1 FROM message_folders mf WHERE mf.account_id=m.account_id AND mf.message_id=m.id AND mf.folder_id=?)");
        args.push(Box::new(folder.clone()));
    } else if let Some(kind) = &folder_kind {
        clauses.push("(CASE WHEN EXISTS(SELECT 1 FROM message_changes c WHERE c.message_id=m.id AND c.field='archive' AND c.value=1) THEN ?='archive' ELSE EXISTS (SELECT 1 FROM message_folders mf JOIN folders f ON f.account_id = mf.account_id AND f.id = mf.folder_id WHERE mf.account_id = m.account_id AND mf.message_id = m.id AND f.kind = ?) END)");
        args.push(Box::new(kind.clone()));
        args.push(Box::new(kind.clone()));
    }
    // Mirrors the UI selector: starred excludes trash; label results exclude trash.
    let exclude_trash = starred_only || query.label_id.is_some();
    if starred_only {
        clauses.push("m.starred = 1");
    }
    if exclude_trash {
        clauses.push("(EXISTS(SELECT 1 FROM message_changes c WHERE c.message_id=m.id AND c.field='archive' AND c.value=1) OR NOT EXISTS (SELECT 1 FROM message_folders mf JOIN folders f ON f.account_id = mf.account_id AND f.id = mf.folder_id WHERE mf.account_id = m.account_id AND mf.message_id = m.id AND f.kind = 'trash'))");
    }
    if let Some(label) = &query.label_id {
        clauses.push("EXISTS (SELECT 1 FROM message_labels ml WHERE ml.message_id = m.id AND ml.label_id = ?)");
        args.push(Box::new(label.clone()));
    }
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if let Some(term) = search {
        clauses.push("m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)");
        args.push(Box::new(fts_expression(term)));
    }
    if let Some(cursor) = &query.cursor {
        let (received_at, id) = cursor_decode(cursor)?;
        clauses.push("(m.received_at < ? OR (m.received_at = ? AND m.id < ?))");
        args.push(Box::new(received_at.clone()));
        args.push(Box::new(received_at));
        args.push(Box::new(id));
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT m.id, m.account_id, COALESCE(m.original_remote_id,m.remote_id), m.thread_id, m.subject, m.sender_name, m.sender_email, m.recipients_json, m.body_text, m.received_at, m.unread, m.starred,
           CASE WHEN EXISTS(SELECT 1 FROM message_changes c WHERE c.message_id=m.id AND c.field='archive' AND c.value=1) THEN 'archive' ELSE (SELECT f.kind FROM message_folders mf JOIN folders f ON f.account_id = mf.account_id AND f.id = mf.folder_id WHERE mf.account_id = m.account_id AND mf.message_id = m.id ORDER BY {KIND_PRIORITY} LIMIT 1) END AS mailbox, m.state_revision
         ,m.content_hash,m.header_message_id,m.locator_verified,m.content_hash_verified,m.attachments_json,m.remote_missing FROM messages m {where_sql} ORDER BY m.received_at DESC, m.id DESC LIMIT ?"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AppError::storage(e.to_string()))?;
    args.push(Box::new(limit + 1));
    let mut rows = stmt
        .query(params_from_iter(args.iter().map(|arg| arg.as_ref())))
        .map_err(|e| AppError::storage(e.to_string()))?;
    let mut items = Vec::new();
    let mut exceeded = false;
    while let Some(row) = rows.next().map_err(|e| AppError::storage(e.to_string()))? {
        if items.len() == limit as usize {
            exceeded = true;
            break;
        }
        let recipients: String = row.get(7).map_err(|e| AppError::storage(e.to_string()))?;
        let body_text: Option<String> = row.get(8).map_err(|e| AppError::storage(e.to_string()))?;
        let id: String = row.get(0).map_err(|e| AppError::storage(e.to_string()))?;
        let received_at: String = row.get(9).map_err(|e| AppError::storage(e.to_string()))?;
        let label_ids = label_ids_for(conn, &id)?;
        items.push(MessageRow {
            body_download:crate::imap_body::status(conn,&id)?,
            folder_ids: {let mut stmt=conn.prepare("SELECT folder_id FROM message_folders WHERE account_id=?1 AND message_id=?2 ORDER BY folder_id").map_err(|_|AppError::storage("无法读取文件夹"))?;let folders=stmt.query_map(params![row.get::<_,String>(1).map_err(|_|AppError::storage("无法读取账号"))?,id],|r|r.get(0)).map_err(|_|AppError::storage("无法读取文件夹"))?.collect::<Result<Vec<_>,_>>().map_err(|_|AppError::storage("文件夹数据损坏"))?;folders},
            attachments: serde_json::from_str(
                &row.get::<_, String>(18)
                    .map_err(|_| AppError::storage("无法读取附件信息"))?,
            )
            .map_err(|_| AppError::storage("附件信息损坏"))?,
            remote_missing: row
                .get(19)
                .map_err(|_| AppError::storage("无法读取远端状态"))?,
            recovery: crate::mail_recovery::status(conn, &id)?,
            content_hash: row.get(14).map_err(|e| AppError::storage(e.to_string()))?,
            locator_verified: row.get(16).map_err(|e| AppError::storage(e.to_string()))?,
            content_hash_verified: row.get(17).map_err(|e| AppError::storage(e.to_string()))?,
            header_message_id: row.get(15).map_err(|e| AppError::storage(e.to_string()))?,
            state_revision: row.get(13).map_err(|e| AppError::storage(e.to_string()))?,
            changes: crate::message_changes::statuses(conn, &id)?,
            draft: None,
            id: id.clone(),
            account_id: row.get(1).map_err(|e| AppError::storage(e.to_string()))?,
            remote_id: row.get(2).map_err(|e| AppError::storage(e.to_string()))?,
            thread_id: row.get(3).map_err(|e| AppError::storage(e.to_string()))?,
            mailbox: row
                .get::<_, Option<String>>(12)
                .map_err(|e| AppError::storage(e.to_string()))?
                .unwrap_or_else(|| "inbox".into()),
            from: FromRow {
                name: row.get(5).map_err(|e| AppError::storage(e.to_string()))?,
                email: row.get(6).map_err(|e| AppError::storage(e.to_string()))?,
            },
            to: serde_json::from_str(&recipients)
                .map_err(|_| AppError::storage("收件人数据损坏"))?,
            subject: row.get(4).map_err(|e| AppError::storage(e.to_string()))?,
            preview: preview_of(body_text.as_deref()),
            body_text: body_text.unwrap_or_default(),
            received_at,
            unread: row
                .get::<_, i64>(10)
                .map_err(|e| AppError::storage(e.to_string()))?
                != 0,
            starred: row
                .get::<_, i64>(11)
                .map_err(|e| AppError::storage(e.to_string()))?
                != 0,
            label_ids,
        });
    }
    let next_cursor = if exceeded {
        let last = items
            .last()
            .ok_or_else(|| AppError::storage("分页状态异常"))?;
        Some(cursor_encode(&last.received_at, &last.id))
    } else {
        None
    };
    Ok(MessagePage { items, next_cursor })
}

pub fn message_by_id(conn: &Connection, account: &str, id: &str) -> Result<MessageRow, AppError> {
    page_messages(
        conn,
        &MessageQuery {
            account_id: Some(account.into()),
            message_id: Some(id.into()),
            ..Default::default()
        },
    )?
    .items
    .into_iter()
    .next()
    .ok_or_else(|| AppError::invalid("邮件不存在或不属于此账号"))
}

fn label_ids_for(conn: &Connection, message_id: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn
        .prepare("SELECT label_id FROM message_labels WHERE message_id = ?1")
        .map_err(|e| AppError::storage(e.to_string()))?;
    let rows = stmt
        .query_map(params![message_id], |row| row.get(0))
        .map_err(|e| AppError::storage(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::storage(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    fn seed_fixture() -> SeedWorkspace {
        let capabilities = serde_json::json!({ "folders": true, "serverLabels": true, "readStateSync": true, "threads": true, "calendar": true });
        let account = |id: &str, provider: &str, email: &str| SeedAccount {
            id: id.into(),
            provider: provider.into(),
            email: email.into(),
            name: format!("账号 {id}"),
            avatar: "A".into(),
            color: "teal".into(),
            status: "demo".into(),
            capabilities: capabilities.clone(),
        };
        let label = |id: &str| SeedLabel {
            id: id.into(),
            name: format!("标签 {id}"),
            color: "teal".into(),
            scope: "local".into(),
        };
        let message =
            |id: &str, account: &str, mailbox: &str, at: &str, body: &str, labels: Vec<&str>| {
                SeedMessage {
                    id: id.into(),
                    account_id: account.into(),
                    remote_id: None,
                    thread_id: id.into(),
                    mailbox: mailbox.into(),
                    from: SeedAddress {
                        name: "发件人".into(),
                        email: "from@example.test".into(),
                    },
                    to: vec!["me@example.test".into()],
                    subject: format!("主题 {id}"),
                    body_text: body.into(),
                    received_at: at.into(),
                    unread: true,
                    starred: false,
                    label_ids: labels.into_iter().map(str::to_string).collect(),
                }
            };
        SeedWorkspace {
            accounts: vec![
                account("a", "imap", "a@example.test"),
                account("b", "imap", "b@example.test"),
            ],
            labels: vec![label("design"), label("life")],
            messages: vec![
                message(
                    "m1",
                    "a",
                    "inbox",
                    "2026-01-03T00:00:00Z",
                    "正文一",
                    vec!["design"],
                ),
                message("m2", "a", "inbox", "2026-01-02T00:00:00Z", "正文二", vec![]),
                message("m3", "a", "sent", "2026-01-01T00:00:00Z", "正文三", vec![]),
                message(
                    "t1",
                    "a",
                    "trash",
                    "2026-01-04T00:00:00Z",
                    "废纸篓邮件",
                    vec!["design"],
                ),
                message(
                    "n1",
                    "b",
                    "inbox",
                    "2026-01-05T00:00:00Z",
                    "另一个账号",
                    vec![],
                ),
            ],
            events: vec![SeedEvent {
                id: "e1".into(),
                account_id: "a".into(),
                message_id: Some("m1".into()),
                title: "待办".into(),
                starts_at: "2026-02-01T00:00:00Z".into(),
                time_zone: "UTC".into(),
                completed: false,
                source: "local".into(),
            }],
        }
    }

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        storage::migrate(&mut conn).unwrap();
        seed_workspace(&mut conn, &seed_fixture()).unwrap();
        conn
    }

    #[test]
    fn reseeding_preserves_demo_read_star_archive_intents() {
        let mut conn = setup();
        crate::message_changes::update(
            &mut conn,
            "a",
            "m1",
            &crate::message_changes::Patch {
                unread: Some(false),
                starred: Some(true),
                archive: Some(true),
            },
        )
        .unwrap();
        seed_workspace(&mut conn, &seed_fixture()).unwrap();
        let saved = message_by_id(&conn, "a", "m1").unwrap();
        assert!(!saved.unread && saved.starred);
        assert_eq!(saved.mailbox, "archive");
        assert!(saved.changes.iter().all(|c| c.state == "local"));
        crate::message_changes::update(
            &mut conn,
            "a",
            "t1",
            &crate::message_changes::Patch {
                starred: Some(true),
                archive: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        for query in [
            MessageQuery {
                mailbox: Some("starred".into()),
                ..Default::default()
            },
            MessageQuery {
                label_id: Some("design".into()),
                ..Default::default()
            },
        ] {
            assert!(page_messages(&conn, &query)
                .unwrap()
                .items
                .iter()
                .any(|message| message.id == "t1" && message.mailbox == "archive"));
        }
    }

    #[test]
    fn refreshing_demo_messages_preserves_persisted_drafts() {
        let mut conn = setup();
        conn.execute("INSERT INTO drafts (id,account_id,subject,body_text,updated_at) VALUES ('draft-retained','a','Keep','Do not delete','2026-11-11T00:00:00Z')", []).unwrap();
        seed_workspace(&mut conn, &seed_fixture()).unwrap();
        let drafts = crate::drafts::list(&conn).unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].body_text, "Do not delete");
    }

    fn all_ids(conn: &Connection, query: &MessageQuery) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor = query.cursor.clone();
        loop {
            let page = page_messages(
                conn,
                &MessageQuery {
                    cursor: cursor.clone(),
                    ..query.clone()
                },
            )
            .unwrap();
            ids.extend(page.items.iter().map(|item| item.id.clone()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        ids
    }

    #[test]
    fn paging_walks_every_message_once_in_descending_order() {
        let conn = setup();
        let ids = all_ids(
            &conn,
            &MessageQuery {
                account_id: Some("a".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(ids, vec!["t1", "m1", "m2", "m3"]);
        let single = page_messages(
            &conn,
            &MessageQuery {
                account_id: Some("a".into()),
                limit: Some(2),
                ..MessageQuery::default()
            },
        )
        .unwrap();
        assert_eq!(single.items.len(), 2);
        assert_eq!(
            single.next_cursor,
            Some(cursor_encode("2026-01-03T00:00:00Z", "m1"))
        );
        let invalid = page_messages(
            &conn,
            &MessageQuery {
                cursor: Some("broken".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(invalid.unwrap_err().code, "invalid");
    }

    #[test]
    fn queries_are_isolated_per_account() {
        let conn = setup();
        let ids = all_ids(
            &conn,
            &MessageQuery {
                account_id: Some("b".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(ids, vec!["n1"]);
    }

    #[test]
    fn folder_starred_and_label_filters_match_the_ui_selector() {
        let conn = setup();
        let inbox = all_ids(
            &conn,
            &MessageQuery {
                mailbox: Some("inbox".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(inbox, vec!["n1", "m1", "m2"]);
        let starred = SeedMessage {
            starred: true,
            ..seed_fixture().messages[0].clone()
        };
        let mut seed = seed_fixture();
        seed.messages[0] = starred;
        let mut conn = Connection::open_in_memory().unwrap();
        storage::migrate(&mut conn).unwrap();
        seed_workspace(&mut conn, &seed).unwrap();
        let starred_ids = all_ids(
            &conn,
            &MessageQuery {
                mailbox: Some("starred".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(starred_ids, vec!["m1"]); // starred flag set, trash excluded despite its design label
        let label_ids = all_ids(
            &conn,
            &MessageQuery {
                label_id: Some("design".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(label_ids, vec!["m1"]);
    }

    #[test]
    fn search_is_sanitized_and_matches_body_content() {
        let conn = setup();
        let hits = all_ids(
            &conn,
            &MessageQuery {
                search: Some("正文二".into()),
                ..MessageQuery::default()
            },
        );
        assert_eq!(hits, vec!["m2"]); // FTS5 phrase match on body text
                                      // Hostile input must never raise an FTS syntax error; separator-only phrases match nothing.
        let hostile = all_ids(
            &conn,
            &MessageQuery {
                search: Some("\" OR 1=1 --".into()),
                ..MessageQuery::default()
            },
        );
        assert!(hostile.is_empty());
        let misses = all_ids(
            &conn,
            &MessageQuery {
                search: Some("不存在的词".into()),
                ..MessageQuery::default()
            },
        );
        assert!(misses.is_empty());
    }

    #[test]
    fn reseeding_replaces_rows_without_duplicates() {
        let mut conn = setup();
        seed_workspace(&mut conn, &seed_fixture()).unwrap();
        let base = load_workspace(&conn).unwrap();
        assert_eq!(base.accounts.len(), 2);
        assert_eq!(base.labels.len(), 2);
        assert_eq!(base.events.len(), 1);
        let ids = all_ids(&conn, &MessageQuery::default());
        assert_eq!(ids.len(), 5);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            5
        );
    }

    #[test]
    fn projection_prefers_inbox_and_dto_fields_survive() {
        let mut seed = seed_fixture();
        seed.messages[1].mailbox = "archive".into();
        // m2 also joins inbox via a second folder membership when seeded again below.
        let mut conn = Connection::open_in_memory().unwrap();
        storage::migrate(&mut conn).unwrap();
        seed_workspace(&mut conn, &seed).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO message_folders (account_id, message_id, folder_id) VALUES ('a', 'm2', 'a:inbox')",
            [],
        ).unwrap();
        let page = page_messages(
            &conn,
            &MessageQuery {
                account_id: Some("a".into()),
                limit: Some(10),
                ..MessageQuery::default()
            },
        )
        .unwrap();
        let m2 = page.items.iter().find(|item| item.id == "m2").unwrap();
        assert_eq!(m2.mailbox, "inbox"); // inbox outranks archive in the deterministic projection
        assert_eq!(m2.to, vec!["me@example.test"]);
        assert!(m2.label_ids.is_empty()); // m2 carries no label in the fixture
        let m1 = page.items.iter().find(|item| item.id == "m1").unwrap();
        assert_eq!(m1.label_ids, vec!["design"]);
        assert!(m2.preview.chars().count() <= PREVIEW_CHARS);
    }

    #[test]
    fn unknown_folder_kind_is_rejected() {
        let mut seed = seed_fixture();
        seed.messages[0].mailbox = "unknown".into();
        let mut conn = Connection::open_in_memory().unwrap();
        storage::migrate(&mut conn).unwrap();
        let error = seed_workspace(&mut conn, &seed).unwrap_err();
        assert_eq!(error.code, "invalid");
    }
}
