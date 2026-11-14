//! Outbox：发送队列与结果语义（docs/SENDING.md）。
//! Message-ID 登记时生成一次、全部尝试复用；DATA 终点号后的不明确结果一律 unknown，
//! 不自动重试、不自动判失败；出口只有用户明确确认或同步后 Sent 目录命中。
use crate::{credentials, error::AppError, imap_transport, mail_oauth, smtp_send};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

const SENT_DOMAIN: &str = "auumail.local";
const KEEP_LOCAL_NOTE: &str = "已按仅保留本机处理；服务器是否投递未被确认，不会自动重试";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub id: String,
    pub draft_id: String,
    pub account_id: String,
    pub message_id: Option<String>,
    pub state: String,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub queued_at: Option<String>,
    pub updated_at: String,
    pub resolved_by: Option<String>,
}

const ENTRY_COLUMNS: &str = "id, draft_id, account_id, message_id, state, attempts, last_error, queued_at, updated_at, resolved_by";

fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        draft_id: row.get(1)?,
        account_id: row.get(2)?,
        message_id: row.get(3)?,
        state: row.get(4)?,
        attempts: row.get::<_, i64>(5)? as u32,
        last_error: row.get(6)?,
        queued_at: row.get(7)?,
        updated_at: row.get(8)?,
        resolved_by: row.get(9)?,
    })
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<Entry>, AppError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM outbox ORDER BY queued_at DESC, id DESC LIMIT 128"
        ))
        .map_err(|_| AppError::storage("无法读取发送队列"))?;
    let entries = statement
        .query_map([], entry)
        .map_err(|_| AppError::storage("无法读取发送队列"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("发送队列数据无法读取"))?;
    Ok(entries)
}

/// 收件人语法：逗号分隔的纯地址（local@domain），拒绝空白、控制字符与多余 @。
pub(crate) fn parse_recipients(raw: &str) -> Result<Vec<String>, AppError> {
    let mut recipients = Vec::new();
    for address in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
        let well_formed = address.len() <= 320
            && !address.chars().any(|c| c.is_whitespace() || c.is_control())
            && address.matches('@').count() == 1;
        let Some((local, domain)) = address.split_once('@').filter(|_| well_formed) else {
            return Err(AppError::invalid(format!("收件人地址无效：{address}")));
        };
        if local.is_empty()
            || domain.is_empty()
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
        {
            return Err(AppError::invalid(format!("收件人地址无效：{address}")));
        }
        recipients.push(address.to_string());
    }
    if recipients.is_empty() {
        return Err(AppError::invalid("请填写至少一个收件人"));
    }
    Ok(recipients)
}

/// RFC2047-B：非 ASCII 文本按字符累积切块，单个 encoded-word 连同标记不超过 75 字符，
/// 词间以折行空格分隔（解码时忽略 encoded-word 之间的空白）。
pub(crate) fn encode_rfc2047(text: &str) -> String {
    // 仅对完全安全的展示字符集保持原样；其他（含 CJK、冒号、@ 等结构字符）一律编码，
    // 防止显示名 / 主题夹带头部注入或结构歧义。
    const SAFE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .'-";
    if text.chars().all(|c| SAFE.contains(c)) && text.len() <= 75 {
        return text.to_string();
    }
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        let candidate = format!("{current}{character}");
        let encoded = format!("=?utf-8?B?{}?=", BASE64.encode(candidate.as_bytes()));
        if encoded.len() > 75 && !current.is_empty() {
            words.push(current);
            current = character.to_string();
        } else {
            current = candidate;
        }
    }
    words.push(current);
    words
        .iter()
        .map(|chunk| format!("=?utf-8?B?{}?=", BASE64.encode(chunk.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

/// RFC5322 组装。头部注入防护：显示名 / 主题中的 CR / LF 替换为空格。
fn fold_base64(data: &str) -> String {
    data.as_bytes()
        .chunks(76)
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "")
        .unwrap_or_default()
        .join("\r\n")
}

fn assemble(
    from_name: &str,
    from_email: &str,
    recipients: &[String],
    subject: &str,
    body_text: &str,
    body_html: Option<&str>,
    message_id: &str,
) -> String {
    let clean = |text: &str| text.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>();
    let safe_name = clean(from_name).trim().to_string();
    let from = if safe_name.is_empty() {
        format!("From: <{from_email}>")
    } else {
        format!("From: {} <{from_email}>", encode_rfc2047(&safe_name))
    };
    let safe_subject = clean(subject).trim().to_string();
    let subject_line = if safe_subject.is_empty() {
        String::new()
    } else {
        format!("Subject: {}\r\n", encode_rfc2047(&safe_subject))
    };
    let text_part = format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
        fold_base64(&BASE64.encode(body_text.as_bytes()))
    );
    let (content_headers, body) = match body_html.filter(|html| !html.trim().is_empty()) {
        Some(html) => {
            let boundary = format!("=_auumail_{}", Uuid::new_v4().simple());
            let html_part = format!(
                "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
                fold_base64(&BASE64.encode(html.as_bytes()))
            );
            (
                format!("Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n"),
                format!("--{boundary}\r\n{text_part}\r\n--{boundary}\r\n{html_part}\r\n--{boundary}--\r\n"),
            )
        }
        None => (
            "Content-Type: text/plain; charset=utf-8\r\n".into(),
            format!("{text_part}\r\n"),
        ),
    };
    format!(
        "{from}\r\nTo: {}\r\n{subject_line}Date: {}\r\nMessage-ID: {message_id}\r\nMIME-Version: 1.0\r\n{content_headers}\r\n{body}",
        recipients.join(", "),
        Utc::now().to_rfc2822(),
    )
}

/// 登记发送：校验修订 / 能力 / 收件人，生成一次 Message-ID 并全程复用。
pub(crate) fn enqueue(conn: &Connection, draft_id: &str, revision: u32) -> Result<Entry, AppError> {
    let draft: Option<(String, u32, String, String, String)> = conn
        .query_row(
            "SELECT id, revision, recipients_json, subject, body_text FROM drafts WHERE id = ?1",
            [draft_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, String>(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取草稿"))?;
    let Some((_, stored_revision, recipients_json, _, _)) = draft else {
        return Err(AppError::invalid("草稿不存在"));
    };
    if stored_revision != revision {
        return Err(AppError::conflict("草稿已被修改，请保存后重新发送"));
    }
    parse_recipients(
        &serde_json::from_str::<Vec<String>>(&recipients_json)
            .map(|items| items.join(", "))
            .map_err(|_| AppError::storage("草稿收件人数据损坏"))?,
    )?;
    let blocked: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = ?1)",
            [draft_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::storage("无法检查发送状态"))?;
    if blocked {
        return Err(AppError::conflict(
            "这封草稿已进入发送流程，请保留当前内容并另建草稿。",
        ));
    }
    let (account_id, profile): (String, String) = conn
        .query_row(
            "SELECT id, profile_json FROM accounts WHERE id = (SELECT account_id FROM drafts WHERE id = ?1)",
            [draft_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::storage("无法读取账号配置"))?;
    let profile: serde_json::Value =
        serde_json::from_str(&profile).map_err(|_| AppError::storage("账号配置损坏"))?;
    if profile["status"] == "demo" {
        return Err(AppError::invalid("演示账号不会连接服务器，也不能发送邮件"));
    }
    let google_oauth = profile.get("oauth").and_then(|oauth| oauth.get("provider")).and_then(|provider| provider.as_str()) == Some("google");
    // Google OAuth 发送走 smtp.gmail.com，不受历史「仅收信」标记限制（OAuth 曾默认只收信）。
    if !google_oauth && profile["receiveOnly"].as_bool().unwrap_or(false) {
        return Err(AppError::invalid("仅收信账号不支持发送，请使用完整 SMTP 账号"));
    }
    if profile["smtpHost"].as_str().unwrap_or("").trim().is_empty() && !google_oauth {
        return Err(AppError::invalid("账号未配置 SMTP 服务器，无法发送"));
    }
    if profile
        .get("oauth")
        .and_then(|value| value.get("provider"))
        .and_then(|provider| provider.as_str())
        == Some("microsoft")
    {
        return Err(AppError::invalid(
            "Microsoft 账号尚未授权 SMTP.Send，发送暂不可用",
        ));
    }
    let message_id = format!(
        "<{}.{}@{SENT_DOMAIN}>",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let entry_id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO outbox (id, account_id, draft_id, state, idempotency_key, message_id, attempts, queued_at, updated_at)
         VALUES (?1, ?2, ?3, 'queued', ?4, ?4, 0, ?5, ?5)",
        params![entry_id, account_id, draft_id, message_id, now()],
    )
    .map_err(|_| AppError::storage("发送队列写入失败"))?;
    current(conn, &entry_id, "queued")
}

struct SendJob {
    entry_id: String,
    account_id: String,
    message_id: String,
    generation: String,
    from_name: String,
    from_email: String,
    smtp_host: String,
    smtp_port: u16,
    auth_user: String,
    oauth: Option<mail_oauth::Config>,
    subject: String,
    body_text: String,
    body_html: String,
    recipients: Vec<String>,
}

/// 短锁快照：取下一个 queued 任务 + 账号 / 草稿信息，并标记 sending。
fn snapshot_job(conn: &Connection, account_id: &str) -> Result<Option<SendJob>, AppError> {
    let Some(row) = conn
        .query_row(
            &format!(
                "SELECT {} FROM outbox WHERE account_id = ?1 AND state = 'queued' ORDER BY queued_at LIMIT 1",
                ENTRY_COLUMNS
            ),
            [account_id],
            entry,
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取发送任务"))?
    else {
        return Ok(None);
    };
    let (generation, display_name, from_email, profile): (String, String, String, String) = conn
        .query_row(
            "SELECT sync_generation, display_name, email, profile_json FROM accounts WHERE id = ?1",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| AppError::storage("无法读取账号配置"))?;
    let profile: serde_json::Value =
        serde_json::from_str(&profile).map_err(|_| AppError::storage("账号配置损坏"))?;
    if profile["status"] == "demo" {
        return Err(AppError::invalid("演示账号不会连接服务器，也不能发送邮件"));
    }
    let draft: Option<(u32, String, String, String, String)> = conn
        .query_row(
            "SELECT revision, recipients_json, subject, body_text, body_html FROM drafts WHERE id = ?1",
            [&row.draft_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取草稿"))?;
    let Some((_, recipients_json, subject, body_text, body_html)) = draft else {
        return Err(AppError::invalid("草稿已不存在，无法发送"));
    };
    let recipients: Vec<String> = serde_json::from_str(&recipients_json)
        .map(|raw: Vec<String>| raw.join(", "))
        .map_err(|_| AppError::storage("草稿收件人数据损坏"))?
        .pipe(|raw| parse_recipients(&raw))?;
    conn.execute(
        "UPDATE outbox SET state = 'sending', attempts = attempts + 1, updated_at = ?2 WHERE id = ?1 AND state = 'queued'",
        params![row.id, now()],
    )
    .map_err(|_| AppError::storage("无法更新发送状态"))?;
    Ok(Some(SendJob {
        entry_id: row.id,
        account_id: account_id.to_string(),
        message_id: row.message_id.unwrap_or_default(),
        generation,
        from_name: display_name,
        from_email,
        smtp_host: profile["smtpHost"].as_str().unwrap_or("").trim().to_string(),
        smtp_port: profile["smtpPort"].as_u64().unwrap_or(465) as u16,
        auth_user: profile["authUser"].as_str().unwrap_or("").to_string(),
        oauth: profile.get("oauth").and_then(|value| {
            serde_json::from_value::<mail_oauth::Config>(value.clone())
                .ok()
                .map(mail_oauth::Config::with_builtin_client_id)
        }),
        subject,
        body_text,
        body_html,
        recipients,
    }))
}

/// Google OAuth 空 SMTP 端口时默认 465 TLS。
fn smtp_port(job: &SendJob, smtp_host: &str) -> u16 {
    if job.smtp_port == 0 || (job.smtp_host.is_empty() && smtp_host == "smtp.gmail.com" && job.smtp_port != 587) {
        465
    } else {
        job.smtp_port
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 短事务写终态；行已被并发改写（重启恢复等）时保持现状。
fn finish(conn: &Connection, entry_id: &str, result: &Result<(), AppError>) -> Result<(), AppError> {
    let (state, error) = match result {
        Ok(()) => ("sent", None),
        Err(error) if error.code == "unknown" => ("unknown", Some(error.message.clone())),
        Err(error) => ("failed", Some(error.message.clone())),
    };
    conn.execute(
        "UPDATE outbox SET state = ?2, last_error = ?3, updated_at = ?4 WHERE id = ?1 AND state = 'sending'",
        params![entry_id, state, error, now()],
    )
    .map_err(|_| AppError::storage("无法写入发送结果"))?;
    Ok(())
}

/// 锁外投递：凭据读取与 SMTP 会话都不持有 DB 锁。
fn deliver(db: &crate::mail_sync::SharedDb, job: &SendJob) -> Result<(), AppError> {
    let google_oauth = matches!(&job.oauth, Some(config) if config.provider == mail_oauth::Provider::Google);
    if job.smtp_host.is_empty() && !google_oauth {
        return Err(AppError::invalid("账号未配置 SMTP 服务器，无法发送"));
    }
    // Google OAuth 账号固定走 smtp.gmail.com（scope https://mail.google.com/ 覆盖 SMTP）。
    let smtp_host = if job.smtp_host.is_empty() { "smtp.gmail.com".to_string() } else { job.smtp_host.clone() };
    let security = if smtp_port(job, &smtp_host) == 587 {
        imap_transport::Security::Starttls
    } else {
        imap_transport::Security::Tls
    };
    let raw = assemble(
        &job.from_name,
        &job.from_email,
        &job.recipients,
        &job.subject,
        &job.body_text,
        Some(job.body_html.as_str()).filter(|html| !html.trim().is_empty()),
        &job.message_id,
    );
    let stream = smtp_send::open(&smtp_host, job.smtp_port, security)?;
    enum PreparedAuth {
        Plain(String),
        Xoauth2(String),
    }
    let prepared = match &job.oauth {
        Some(config) => {
            if config.provider == mail_oauth::Provider::Microsoft {
                return Err(AppError::invalid(
                    "Microsoft 账号尚未授权 SMTP.Send，发送暂不可用",
                ));
            }
            if config.provider == mail_oauth::Provider::Google
                && !job.smtp_host.is_empty()
                && !job.smtp_host.eq_ignore_ascii_case("smtp.gmail.com")
            {
                return Err(AppError::invalid("Google OAuth 账号只能使用 smtp.gmail.com 发送"));
            }
            PreparedAuth::Xoauth2(mail_oauth::access(
                db,
                &job.account_id,
                &job.generation,
                &job.auth_user,
                config,
            )?)
        }
        None => PreparedAuth::Plain(credentials::with_account("imap", &job.account_id, || {
            credentials::secret_for("imap", &job.account_id)
        })?),
    };
    let auth = match &prepared {
        PreparedAuth::Plain(secret) => smtp_send::SendAuth::Plain { user: &job.auth_user, secret },
        PreparedAuth::Xoauth2(token) => smtp_send::SendAuth::Xoauth2 { user: &job.auth_user, token },
    };
    smtp_send::send_session(stream, &auth, &job.from_email, &job.recipients, raw.as_bytes())
}

pub(crate) fn run(db: &crate::mail_sync::SharedDb, account_id: &str) -> Result<bool, AppError> {
    let mut processed = false;
    loop {
        // 入队后才发现的问题（草稿被删、收件人损坏等）不能让行卡在 queued：直接判 failed 继续。
        let snapshot = {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            snapshot_job(&conn, account_id)
        };
        let job = match snapshot {
            Ok(job) => job,
            Err(error) => {
                let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
                if let Some(row) = conn
                    .query_row(
                        &format!("SELECT {} FROM outbox WHERE account_id = ?1 AND state = 'queued' ORDER BY queued_at LIMIT 1", ENTRY_COLUMNS),
                        [account_id],
                        entry,
                    )
                    .optional()
                    .map_err(|_| AppError::storage("无法读取发送任务"))?
                {
                    conn.execute(
                        "UPDATE outbox SET state = 'failed', last_error = ?3, updated_at = ?2 WHERE id = ?1",
                        params![row.id, now(), error.message],
                    )
                    .map_err(|_| AppError::storage("无法写入发送状态"))?;
                    processed = true;
                    continue;
                }
                return Err(error);
            }
        };
        let Some(job) = job else { break };
        let result = deliver(db, &job);
        {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            finish(&conn, &job.entry_id, &result)?;
        }
        processed = true;
    }
    Ok(processed)
}

pub(crate) fn recover(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "UPDATE outbox SET state = 'unknown', last_error = '上次发送中断，结果待核对；不会自动重发', updated_at = ?1 WHERE state = 'sending'",
        params![now()],
    )
    .map_err(|_| AppError::storage("无法恢复发送队列"))?;
    Ok(())
}

/// 同步提交后核对：Sent 目录出现相同 Message-ID → unknown 自动转 sent；
/// 未命中保持 unknown（服务器不保存 Sent 副本不构成未送达证据）。
pub(crate) fn resolve_unknown_after_sync(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "UPDATE outbox SET state = 'sent', resolved_by = 'sent-folder', last_error = NULL, updated_at = ?1
         WHERE state = 'unknown' AND message_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM messages m
           JOIN message_folders mf ON mf.account_id = m.account_id AND mf.message_id = m.id
           JOIN folders f ON f.account_id = mf.account_id AND f.id = mf.folder_id AND f.kind = 'sent'
           WHERE m.header_message_id = outbox.message_id AND m.account_id = outbox.account_id)",
        params![now()],
    )
    .map_err(|_| AppError::storage("无法核对已发送邮件"))?;
    Ok(())
}

/// 用户出口：unknown → 仅保留本机（failed，不自动重试）或确认重试（queued，Message-ID 不变）。
pub(crate) fn resolve(conn: &Connection, entry_id: &str, action: &str) -> Result<Entry, AppError> {
    let state = match action {
        "keep-local" => {
            conn.execute(
                "UPDATE outbox SET state = 'failed', resolved_by = 'keep-local', last_error = ?3, updated_at = ?2 WHERE id = ?1 AND state = 'unknown'",
                params![entry_id, now(), KEEP_LOCAL_NOTE],
            )
            .map_err(|_| AppError::storage("无法写入发送状态"))?;
            "failed"
        }
        "retry" => {
            conn.execute(
                "UPDATE outbox SET state = 'queued', last_error = NULL, updated_at = ?2 WHERE id = ?1 AND state = 'unknown'",
                params![entry_id, now()],
            )
            .map_err(|_| AppError::storage("无法写入发送状态"))?;
            "queued"
        }
        _ => return Err(AppError::invalid("未知的处理方式")),
    };
    current(conn, entry_id, state)
}

pub(crate) fn retry(conn: &Connection, entry_id: &str) -> Result<Entry, AppError> {
    conn.execute(
        "UPDATE outbox SET state = 'queued', last_error = NULL, updated_at = ?2 WHERE id = ?1 AND state = 'failed'",
        params![entry_id, now()],
    )
    .map_err(|_| AppError::storage("无法重排发送任务"))?;
    current(conn, entry_id, "queued")
}

pub(crate) fn cancel(conn: &Connection, entry_id: &str) -> Result<Entry, AppError> {
    // 仅排队中可取消：取回条目后删除，草稿解除占用恢复可编辑。
    let entry = current(conn, entry_id, "queued")?;
    let deleted = conn
        .execute("DELETE FROM outbox WHERE id = ?1 AND state = 'queued'", params![entry_id])
        .map_err(|_| AppError::storage("无法取消发送任务"))?;
    if deleted == 0 {
        return Err(AppError::conflict("该任务已开始发送或已完成，不能取消"));
    }
    Ok(entry)
}

fn current(conn: &Connection, entry_id: &str, expected: &str) -> Result<Entry, AppError> {
    let row = conn
        .query_row(
            &format!("SELECT {ENTRY_COLUMNS} FROM outbox WHERE id = ?1"),
            [entry_id],
            entry,
        )
        .optional()
        .map_err(|e| AppError::storage(format!("无法读取发送任务: {e}")))?;
    let entry = row.ok_or_else(|| AppError::storage("发送任务已不存在"))?;
    if entry.state != expected {
        return Err(AppError::conflict("发送状态已变化，请刷新后查看"));
    }
    Ok(entry)
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        db.execute_batch(
            "INSERT INTO accounts (id, provider, email, display_name, created_at, profile_json) VALUES
             ('real', 'imap', 'me@example.test', '测试发件人', '2026-11-14T00:00:00Z',
              '{\"status\":\"connected\",\"smtpHost\":\"smtp.example.test\",\"smtpPort\":465,\"authUser\":\"me@example.test\",\"receiveOnly\":false}');",
        )
        .unwrap();
        db.execute_batch("INSERT INTO drafts (id, account_id, recipients_json, subject, body_text, reply_to_id, updated_at, revision) VALUES
          ('draft-1', 'real', '[\"friend@example.test\"]', '你好', '正文', NULL, '2026-11-14T00:00:00Z', 3);")
            .unwrap();
        db
    }

    #[test]
    fn enqueue_records_message_id_and_blocks_duplicates() {
        let conn = db();
        let first = enqueue(&conn, "draft-1", 3).unwrap();
        assert_eq!(first.state, "queued");
        assert!(first.message_id.as_deref().unwrap().starts_with('<'));
        assert_eq!(first.queued_at.as_deref(), Some(first.updated_at.as_str()));
        let again = enqueue(&conn, "draft-1", 3).unwrap_err();
        assert_eq!(again.code, "conflict");
        // 修订不匹配拒绝；Message-ID 必须保持不变（幂等）。
        assert_eq!(enqueue(&conn, "draft-1", 9).unwrap_err().code, "conflict");
        assert_eq!(list(&conn).unwrap()[0].message_id, first.message_id);
    }

    #[test]
    fn receive_only_and_microsoft_and_bad_recipients_are_rejected_at_enqueue() {
        let conn = db();
        // Google OAuth 即使带历史「仅收信」标记也可发送（走 smtp.gmail.com）。
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.receiveOnly', json('true')), '$.oauth', json('{\"provider\":\"google\",\"clientId\":\"id\",\"tenant\":\"common\"}')) WHERE id = 'real'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap().state, "queued");
        conn.execute("DELETE FROM outbox", []).unwrap();
        // 非 OAuth 的仅收信账号仍然拒绝。
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.receiveOnly', json('true')), '$.oauth', json('null')) WHERE id = 'real'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "invalid");
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.receiveOnly', json('false')), '$.oauth', json('{\"provider\":\"microsoft\",\"clientId\":\"id\",\"tenant\":\"common\"}')) WHERE id = 'real'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().message, "Microsoft 账号尚未授权 SMTP.Send，发送暂不可用");
        conn.execute("UPDATE drafts SET recipients_json = '[\"not-an-address\"]'", []).unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "invalid");
    }

    #[test]
    fn draft_revision_mismatch_blocks_enqueue() {
        let conn = db();
        assert_eq!(enqueue(&conn, "draft-1", 99).unwrap_err().code, "conflict");
    }

    #[test]
    fn google_oauth_without_smtp_host_still_enqueues() {
        // OAuth 账号创建时只填 IMAP：发送默认 smtp.gmail.com，不能因空 SMTP 主机被拒。
        let conn = db();
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.oauth', json('{\"provider\":\"google\",\"clientId\":\"id\",\"tenant\":\"common\"}')), '$.smtpHost', '') WHERE id = 'real'",
            [],
        )
        .unwrap();
        let entry = enqueue(&conn, "draft-1", 3).unwrap();
        assert_eq!(entry.state, "queued");
    }

    #[test]
    fn run_marks_credential_failure_failed_and_keeps_draft_locked() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        let db = std::sync::Arc::new(std::sync::Mutex::new(conn));
        // 密码凭据缺失（测试环境没有 keyring）：任务转 failed，错误不含机密。
        let processed = run(&db, "real").unwrap();
        assert!(processed);
        let conn = db.lock().unwrap();
        let entries = list(&conn).unwrap();
        assert_eq!(entries[0].state, "failed");
        assert!(entries[0].last_error.is_some());
        assert_eq!(entries[0].attempts, 1);
        // 草稿仍被 outbox 占用（drafts::write 的守卫覆盖），不会被意外改写。
        let locked: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = 'draft-1')", [], |r| r.get(0))
            .unwrap();
        assert!(locked);
    }

    #[test]
    fn finish_maps_result_codes_to_states() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        conn.execute("UPDATE outbox SET state = 'sending', attempts = 1", []).unwrap();
        finish(&conn, &list(&conn).unwrap()[0].id, &Ok(())).unwrap();
        assert_eq!(list(&conn).unwrap()[0].state, "sent");
        conn.execute("UPDATE outbox SET state = 'sending'", []).unwrap();
        finish(
            &conn,
            &list(&conn).unwrap()[0].id,
            &Err(AppError { code: "unknown", message: "未确认".into(), retryable: false }),
        )
        .unwrap();
        let entry = &list(&conn).unwrap()[0];
        assert_eq!(entry.state, "unknown");
        assert_eq!(entry.last_error.as_deref(), Some("未确认"));
        conn.execute("UPDATE outbox SET state = 'sending'", []).unwrap();
        finish(&conn, &list(&conn).unwrap()[0].id, &Err(AppError::invalid("拒绝"))).unwrap();
        assert_eq!(list(&conn).unwrap()[0].state, "failed");
    }

    #[test]
    fn recover_turns_interrupted_sending_into_unknown() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        conn.execute("UPDATE outbox SET state = 'sending'", []).unwrap();
        recover(&conn).unwrap();
        let entry = &list(&conn).unwrap()[0];
        assert_eq!(entry.state, "unknown");
        assert!(entry.last_error.as_deref().unwrap().contains("中断"));
        // queued 与终态不受影响。
        enqueue(&conn, "draft-1", 3).unwrap_err();
    }

    #[test]
    fn unknown_resolves_via_sent_folder_or_user_exits() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        let message_id = list(&conn).unwrap()[0].message_id.clone().unwrap();
        conn.execute("UPDATE outbox SET state = 'unknown'", []).unwrap();
        // 没有命中时保持 unknown。
        resolve_unknown_after_sync(&conn).unwrap();
        assert_eq!(list(&conn).unwrap()[0].state, "unknown");
        // 服务器 Sent 目录出现相同 Message-ID → 自动 resolved-sent。
        conn.execute(
            "INSERT INTO folders (id, account_id, remote_id, name, kind) VALUES ('f1', 'real', 'sent', 'Sent', 'sent')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, sender_name, sender_email, received_at, header_message_id)
             VALUES ('m-sent', 'real', 'r1', 't', '你好', '测试发件人', 'me@example.test', '2026-11-14T00:00:00Z', ?1)",
            params![message_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_folders (account_id, message_id, folder_id) VALUES ('real', 'm-sent', 'f1')",
            [],
        )
        .unwrap();
        resolve_unknown_after_sync(&conn).unwrap();
        let entry = &list(&conn).unwrap()[0];
        assert_eq!(entry.state, "sent");
        assert_eq!(entry.resolved_by.as_deref(), Some("sent-folder"));
        assert_eq!(entry.last_error, None);
    }

    #[test]
    fn assembly_encodes_cjk_headers_and_folds_body() {
        let raw = assemble(
            "测试发件人",
            "me@example.test",
            &["friend@example.test".into()],
            "你好 · 收据",
            "第一行\n第二行",
            None,
            "<abc@auumail.local>",
        );
        assert!(raw.starts_with("From: =?utf-8?B?"));
        assert!(raw.contains("\r\nTo: friend@example.test\r\n"));
        assert!(raw.contains("Subject: =?utf-8?B?"));
        assert!(raw.contains("Date: "));
        assert!(raw.contains("Message-ID: <abc@auumail.local>"));
        assert!(raw.contains("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
        assert!(!raw.contains("multipart"));
        // base64 正文按 76 字符折行（取最后一段双 CRLF 之后的内容）。
        let body = raw.split("\r\n\r\n").last().unwrap();
        assert!(body.lines().all(|line| line.len() <= 76));
        let decoded = BASE64.decode(body.replace("\r\n", "")).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "第一行\n第二行");
    }

    #[test]
    fn html_body_becomes_multipart_alternative() {
        let raw = assemble(
            "me",
            "me@example.test",
            &["friend@example.test".into()],
            "rich",
            "plain text",
            Some("<p>rich <b>text</b></p>"),
            "<abc@auumail.local>",
        );
        assert!(raw.contains("Content-Type: multipart/alternative; boundary=\"=_auumail_"));
        assert!(raw.contains("Content-Type: text/plain; charset=utf-8"));
        assert!(raw.contains("Content-Type: text/html; charset=utf-8"));
        let boundary = raw.split("boundary=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        assert_eq!(raw.matches(&format!("--{boundary}")).count(), 3, "开边界两次 + 闭边界一次");
        assert!(raw.trim_end().ends_with("--"));
        // 两个部分都能独立解码。
        let encoded = raw
            .split("text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        let decoded = String::from_utf8(BASE64.decode(encoded).unwrap()).unwrap();
        assert_eq!(decoded, "<p>rich <b>text</b></p>");
        // 纯文本部分保留。
        assert!(raw.contains("cGxhaW4gdGV4dA==") || raw.contains(&BASE64.encode(b"plain text")));
    }

    #[test]
    fn header_injection_is_neutralized() {
        let raw = assemble(
            "evil\r\nBcc: x@example.test",
            "me@example.test",
            &["a@example.test".into()],
            "subject\r\nBcc: y@example.test",
            "body",
            None,
            "<abc@auumail.local>",
        );
        assert!(!raw.contains("Bcc:"), "CR/LF 必须被中和，头部不得注入");
        assert!(raw.matches("Message-ID:").count() == 1);
    }

    #[test]
    fn recipients_are_parsed_strictly() {
        assert_eq!(
            parse_recipients("a@example.test, b@example.test").unwrap(),
            vec!["a@example.test".to_string(), "b@example.test".to_string()]
        );
        assert!(parse_recipients("no-at-mark").is_err());
        assert!(parse_recipients("a@b@example.test").is_err());
        assert!(parse_recipients("a @example.test").is_err());
        assert!(parse_recipients("a@localhost").is_err());
        assert!(parse_recipients("").is_err());
    }

    #[test]
    fn user_exits_move_unknown_to_failed_or_back_to_queued() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        // 模拟一次真实投递尝试后的 unknown（attempts 已 +1）。
        conn.execute("UPDATE outbox SET state = 'unknown', attempts = 1", []).unwrap();
        let entry = resolve(&conn, &list(&conn).unwrap()[0].id, "keep-local").unwrap();
        assert_eq!(entry.state, "failed");
        assert_eq!(entry.resolved_by.as_deref(), Some("keep-local"));
        assert!(entry.last_error.as_deref().unwrap().contains("仅保留本机"));
        let entry = retry(&conn, &entry.id).unwrap();
        assert_eq!(entry.state, "queued");
        assert_eq!(entry.attempts, 1, "重试不重置已尝试次数");
        assert_eq!(entry.last_error, None);
    }

    #[test]
    fn cancel_only_removes_queued_entries() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        let entry = list(&conn).unwrap().remove(0);
        conn.execute("UPDATE outbox SET state = 'sending'", []).unwrap();
        assert_eq!(cancel(&conn, &entry.id).unwrap_err().code, "conflict");
        conn.execute("UPDATE outbox SET state = 'queued'", []).unwrap();
        let cancelled = cancel(&conn, &entry.id).unwrap();
        assert_eq!(cancelled.state, "queued");
        let remaining: i64 = conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0)).unwrap();
        assert_eq!(remaining, 0);
    }
}
