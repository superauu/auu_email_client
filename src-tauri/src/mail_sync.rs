//! Snapshot -> unlocked IMAP/parse -> bounded, guarded SQLite transactions.
//! No database guard is held while reading credentials, DNS/TLS, IMAP or MIME data.
use crate::{credentials, error::AppError, imap_writeback, mail_recovery, message_changes};
use mail_parser::{MessageParser, MimeHeaders};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Mutex},
};

pub type SharedDb = Arc<Mutex<Connection>>;
#[cfg(test)]
#[path = "mail_sync_extended_tests.rs"]
mod extended_tests;
#[path = "imap_live.rs"]
mod live;
const INITIAL_WINDOW: u32 = 50;
const FETCH_UID_SPAN: u32 = 100;
const SYNCED_KINDS: [&str; 3] = ["inbox", "sent", "archive"];

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub more_mail: bool,
    pub refresh_needed: bool,
    pub retry_needed: bool,
    pub listening: bool,
    pub has_more: bool,
    pub folders: usize,
    pub fetched_messages: usize,
    pub synced_at: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cursor {
    uid_validity: u32,
    last_uid: u32,
}

struct SyncPlan {
    oauth: Option<crate::mail_oauth::Config>,
    force_full: bool,
    condstore: bool,
    account_id: String,
    generation: String,
    host: String,
    port: u16,
    security: crate::imap_transport::Security,
    auth_user: String,
    cursors: HashMap<String, Option<Cursor>>,
    cache_root: std::path::PathBuf,
    settings: crate::imap_settings::Settings,
}

pub(crate) struct ParsedMessage {
    pub(crate) content_hash: String,
    pub(crate) header_message_id: Option<String>,
    uid: u32,
    subject: String,
    sender_name: String,
    sender_email: String,
    recipients_json: String,
    body_text: String,
    received_at: String,
    unread: bool,
    starred: bool,
    pub(crate) raw_cache_key: Option<String>,
    pub(crate) attachments: Vec<Attachment>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attachment {
    pub index: usize,
    pub name: String,
    pub size: usize,
    pub media_type: String,
}

fn folder_kind(name: &str) -> &'static str {
    let name = name.to_lowercase();
    if name == "inbox" {
        "inbox"
    } else if name.contains("sent") || name.contains("已发送") {
        "sent"
    } else if name.contains("draft") || name.contains("草稿") {
        "drafts"
    } else if name.contains("archive") || name.contains("归档") {
        "archive"
    } else if name.contains("trash") || name.contains("deleted") || name.contains("废纸") {
        "trash"
    } else {
        "other"
    }
}

fn read_cursor(conn: &Connection, id: &str, kind: &str) -> Result<Option<Cursor>, AppError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT cursor FROM sync_states WHERE account_id = ?1 AND scope = ?2",
            params![id, format!("imap:{kind}")],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取同步游标"))?
        .flatten();
    raw.map(|text| {
        serde_json::from_str::<Cursor>(&text)
            .map_err(|_| AppError::storage("同步游标无效，请保留数据后检查"))
    })
    .transpose()
}

fn prepare(conn: &Connection, id: &str) -> Result<SyncPlan, AppError> {
    let (profile, generation): (Option<String>, String) = conn.query_row(
        "SELECT profile_json, sync_generation FROM accounts WHERE id = ?1 AND provider = 'imap'", [id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|_| AppError::invalid("找不到可同步的 IMAP 账号"))?;
    let profile: serde_json::Value = profile
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| AppError::invalid("账号缺少服务器信息"))?;
    if profile["status"] == "demo" {
        return Err(AppError::invalid("演示账号不会连接邮件服务器"));
    }
    let field = |key: &str| {
        profile[key]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| AppError::invalid("账号缺少 IMAP 服务器或登录用户"))
    };
    let port = profile["imapPort"]
        .as_u64()
        .filter(|port| (1..=65535).contains(port))
        .ok_or_else(|| AppError::invalid("账号缺少有效的 IMAP 端口"))? as u16;
    Ok(SyncPlan {
        oauth: profile
            .get("oauth")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|_| AppError::invalid("OAuth 配置无效"))?,
        force_full: conn
            .query_row(
                "SELECT origin FROM imap_sync_requests WHERE account_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| AppError::storage("无法读取同步请求来源"))?
            .is_none_or(|origin| origin == "manual"),
        condstore: false,
        account_id: id.into(),
        generation,
        host: field("imapHost")?,
        port,
        security: profile
            .get("imapSecurity")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|_| AppError::invalid("IMAP 加密设置无效"))?
            .unwrap_or_default(),
        auth_user: field("authUser")?,
        cursors: {
            let mut stmt=conn.prepare("SELECT scope,cursor FROM sync_states WHERE account_id=?1 AND scope LIKE 'imap:%'").map_err(|_|AppError::storage("无法读取文件夹游标"))?;
            let mut result = HashMap::new();
            for row in stmt
                .query_map([id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .map_err(|_| AppError::storage("无法读取文件夹游标"))?
            {
                let (scope, json) = row.map_err(|_| AppError::storage("文件夹游标损坏"))?;
                let cursor = json
                    .map(|v| {
                        serde_json::from_str(&v).map_err(|_| AppError::storage("文件夹游标损坏"))
                    })
                    .transpose()?;
                result.insert(scope.trim_start_matches("imap:").to_owned(), cursor);
            }
            for kind in SYNCED_KINDS {
                result.entry(kind.into()).or_insert(None);
            }
            result
        },
        cache_root: crate::mail_cache::root(conn)?,
        settings: crate::imap_settings::read(conn, id)?,
    })
}

pub fn validate_accounts(db: &SharedDb, ids: &[String]) -> Result<(), AppError> {
    let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    for id in ids {
        prepare(&conn, id)?;
    }
    Ok(())
}

fn ensure_current(conn: &Connection, plan: &SyncPlan) -> Result<(), AppError> {
    let generation: Option<String> = conn
        .query_row(
            "SELECT sync_generation FROM accounts WHERE id = ?1",
            [&plan.account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法核对账号状态"))?;
    if generation.as_deref() != Some(&plan.generation) {
        return Err(AppError::conflict(
            "账号已被删除或连接设置已改变，本次同步已停止。",
        ));
    }
    Ok(())
}

pub(crate) fn sync_account_generation(
    db: &SharedDb,
    id: &str,
    generation: &str,
) -> Result<SyncOutcome, AppError> {
    sync_account_impl(db, id, Some(generation))
}
fn sync_account_impl(
    db: &SharedDb,
    id: &str,
    generation: Option<&str>,
) -> Result<SyncOutcome, AppError> {
    let plan = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        prepare(&conn, id)?
    };
    if generation.is_some_and(|g| g != plan.generation) {
        return Err(AppError::conflict("账号连接代次已改变，旧同步请求未执行"));
    }
    crate::sync_control::check()?;
    live::run(db, plan)
}
fn open_session(
    db: &SharedDb,
    plan: &SyncPlan,
) -> Result<(live::LiveSession, imap_writeback::Trace), AppError> {
    let secret = if let Some(config) = &plan.oauth {
        if !plan.host.eq_ignore_ascii_case(config.host())
            || plan.port != 993
            || plan.security != crate::imap_transport::Security::Tls
        {
            return Err(AppError::invalid(
                "OAuth 仅向所选提供商的受信任 IMAP 主机发送令牌",
            ));
        }
        crate::mail_oauth::access(
            db,
            &plan.account_id,
            &plan.generation,
            &plan.auth_user,
            config,
        )?
    } else {
        credentials::with_account("imap", &plan.account_id, || {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            ensure_current(&conn, plan)?;
            drop(conn);
            credentials::secret_for("imap", &plan.account_id)
        })?
    };
    let tls = crate::imap_transport::connect(&plan.host, plan.port, plan.security)?;
    let (stream, trace) = imap_writeback::TracedStream::new(tls);
    if plan.oauth.is_some() {
        return crate::mail_oauth::login(stream, &plan.auth_user, &secret)
            .map(|session| (session, trace));
    }
    let session = imap::Client::new(stream)
        .login(&plan.auth_user, &secret)
        .map_err(|(error, _)| match error {
            imap::Error::No(_) | imap::Error::Bad(_) => {
                AppError::auth("IMAP 服务器拒绝了登录，请核对账号密码")
            }
            _ => crate::imap_limits::network(error, "IMAP 会话未能建立"),
        })?;
    Ok((session, trace))
}

#[cfg(test)]
fn sync_with_session<S: Read + Write>(
    db: &SharedDb,
    plan: &SyncPlan,
    session: imap::Session<S>,
) -> Result<SyncOutcome, AppError> {
    sync_with_fetcher(db, plan, session, None, tests::fixture_fetch)
}
type Fetcher<S> = fn(
    &mut imap::Session<S>,
    u32,
    u32,
    &str,
    u32,
    &std::path::Path,
) -> Result<Vec<ParsedMessage>, AppError>;
#[cfg(test)]
fn sync_with_session_trace<S: Read + Write>(
    db: &SharedDb,
    plan: &SyncPlan,
    session: imap::Session<S>,
    trace: Option<&imap_writeback::Trace>,
) -> Result<SyncOutcome, AppError> {
    sync_with_fetcher(db, plan, session, trace, crate::imap_download::fetch_range)
}
#[cfg(test)]
fn sync_with_fetcher<S: Read + Write>(
    db: &SharedDb,
    plan: &SyncPlan,
    mut session: imap::Session<S>,
    trace: Option<&imap_writeback::Trace>,
    fetch: Fetcher<S>,
) -> Result<SyncOutcome, AppError> {
    let result = sync_session(db, plan, &mut session, trace, fetch);
    let _ = session.logout();
    result
}
fn sync_session<S: Read + Write>(
    db: &SharedDb,
    plan: &SyncPlan,
    session: &mut imap::Session<S>,
    trace: Option<&imap_writeback::Trace>,
    fetch: Fetcher<S>,
) -> Result<SyncOutcome, AppError> {
    sync_session_mode(db, plan, session, trace, fetch, true)
}
fn sync_session_mode<S: Read + Write>(
    db: &SharedDb,
    plan: &SyncPlan,
    session: &mut imap::Session<S>,
    trace: Option<&imap_writeback::Trace>,
    fetch: Fetcher<S>,
    full: bool,
) -> Result<SyncOutcome, AppError> {
    let listed = session
        .list(Some(""), Some("*"))
        .map_err(|error| crate::imap_limits::network(error, "无法读取邮箱文件夹列表"))?;
    let selectable = |name: &&imap::types::Name| {
        !name
            .attributes()
            .iter()
            .any(|a| matches!(a, imap::types::NameAttribute::NoSelect))
    };
    let special:Vec<String> = listed.iter().filter(selectable).filter(|name| name.attributes().iter().any(|a| matches!(a,imap::types::NameAttribute::Custom(value) if value.eq_ignore_ascii_case("\\Archive")))).map(|name|name.name().to_owned()).collect();
    let candidates: Vec<String> = if special.is_empty() {
        listed
            .iter()
            .filter(selectable)
            .filter(|name| folder_kind(name.name()) == "archive")
            .map(|name| name.name().to_owned())
            .collect()
    } else {
        special
    };
    let archive = (candidates.len() == 1).then(|| candidates[0].clone());
    let names: Vec<String> = listed
        .iter()
        .filter(selectable)
        .map(|name| name.name().to_owned())
        .collect();
    drop(listed);
    {
        let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let tx = conn
            .transaction()
            .map_err(|_| AppError::storage("无法开始文件夹更新"))?;
        ensure_current(&tx, plan)?;
        tx.execute(
            "UPDATE imap_folder_state SET available=0 WHERE account_id=?1",
            [&plan.account_id],
        )
        .map_err(|_| AppError::storage("无法更新文件夹状态"))?;
        for name in &names {
            tx.execute("INSERT INTO folders (id,account_id,remote_id,name,kind) VALUES (?1,?2,?3,?5,?4)
              ON CONFLICT(account_id,remote_id) DO UPDATE SET name=excluded.name,kind=excluded.kind",
              params![format!("{}:folder:{name}", plan.account_id), plan.account_id, name, if archive.as_ref()==Some(name){"archive"}else{folder_kind(name)},crate::imap_settings::display_name(name)])
              .map_err(|_| AppError::storage("无法保存文件夹"))?;
            tx.execute("INSERT INTO imap_folder_state(account_id,remote_id,available) VALUES (?1,?2,1) ON CONFLICT(account_id,remote_id) DO UPDATE SET available=1",params![plan.account_id,name]).map_err(|_|AppError::storage("无法更新文件夹状态"))?;
        }
        tx.execute("UPDATE messages SET remote_missing=1,state_revision=state_revision+1 WHERE account_id=?1 AND remote_missing=0 AND remote_generation=?2 AND EXISTS(SELECT 1 FROM imap_folder_state s WHERE s.account_id=messages.account_id AND s.remote_id=messages.remote_folder AND s.available=0)",params![plan.account_id,plan.generation]).map_err(|_|AppError::storage("无法保留不可用目录缓存"))?;
        tx.execute("UPDATE message_changes SET state='conflict',error='原文件夹已不可用，本机修改保留，请核对目录。' WHERE account_id=?1 AND state IN ('queued','failed') AND EXISTS(SELECT 1 FROM messages m WHERE m.id=message_changes.message_id AND m.remote_missing=1)",[&plan.account_id]).map_err(|_|AppError::storage("无法保留目录冲突"))?;
        tx.commit()
            .map_err(|_| AppError::storage("文件夹更新未完成"))?;
    }
    let mut outcome = SyncOutcome {
        refresh_needed: true,
        folders: names.len(),
        warnings: if plan.force_full || plan.settings.auto_sync_seconds > 0 {
            imap_writeback::flush(db, &plan.account_id, session, archive.as_deref(), trace)?
        } else {
            vec![]
        },
        ..SyncOutcome::default()
    };
    outcome.warnings.extend(mail_recovery::process(
        db,
        &plan.account_id,
        session,
        &names,
    )?);
    outcome
        .warnings
        .extend(crate::imap_body::process(db, &plan.account_id, session)?);
    let mut targets = vec![];
    for kind in SYNCED_KINDS {
        let remote = if kind == "archive" {
            archive.as_ref()
        } else {
            names.iter().find(|name| folder_kind(name) == kind)
        };
        let Some(remote) = remote else {
            continue;
        };
        targets.push((kind.to_owned(), remote.clone()));
    }
    if plan.settings.sync_all_folders {
        for name in &names {
            if !targets.iter().any(|(_, selected)| selected == name) {
                targets.push((crate::imap_settings::scope(name), name.clone()));
            }
        }
    }
    for (kind, remote) in targets.into_iter().filter(|_| full) {
        match sync_folder(db, plan, session, &kind, &remote, fetch) {
            Ok((count, more)) => {
                outcome.fetched_messages += count;
                outcome.has_more |= more;
                outcome.more_mail |= more;
            }
            Err(error) if kind != "inbox" && error.code == "network" => {
                outcome.retry_needed = true;
                outcome
                    .warnings
                    .push(format!("{remote} 文件夹同步失败：{}", error.message));
            }
            Err(error) => return Err(error),
        }
    }
    {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let remaining:i64=conn.query_row("SELECT COUNT(*) FROM message_changes WHERE account_id=?1 AND state IN ('queued','failed')",[&plan.account_id],|r|r.get(0)).map_err(|_|AppError::storage("无法核对剩余修改"))?;
        if remaining > 0 {
            outcome
                .warnings
                .push(format!("还有 {remaining} 项本机修改等待后续同步回写。"));
        }
        let pending:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM message_recovery WHERE account_id=?1 AND state='queued') OR EXISTS(SELECT 1 FROM imap_body_requests WHERE account_id=?1 AND state='queued')",[&plan.account_id],|r|r.get(0)).map_err(|_|AppError::storage("无法读取剩余只读请求"))?;
        outcome.has_more |= pending;
    }
    if outcome.has_more {
        outcome
            .warnings
            .push("本轮完整批次已保存，仍有邮件范围等待继续同步。".into());
    }
    outcome.synced_at = crate::now_utc();
    Ok(outcome)
}
pub(crate) fn forget_session(id: &str) {
    live::forget(id);
}

pub(crate) fn parse_message(fetch: &imap::types::Fetch) -> Result<ParsedMessage, AppError> {
    let uid = fetch
        .uid
        .ok_or_else(|| AppError::network("邮件响应缺少 UID，本批次未提交"))?;
    let raw = fetch
        .body()
        .ok_or_else(|| AppError::network("邮件响应缺少正文，本批次未提交"))?;
    parse_raw(
        uid,
        raw,
        fetch
            .internal_date()
            .map(|date| date.naive_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(crate::now_utc),
        !fetch.flags().contains(&imap::types::Flag::Seen),
        fetch.flags().contains(&imap::types::Flag::Flagged),
    )
}
pub(crate) fn parse_raw(
    uid: u32,
    raw: &[u8],
    received_at: String,
    unread: bool,
    starred: bool,
) -> Result<ParsedMessage, AppError> {
    let parsed = MessageParser::default().parse(raw);
    let address = parsed
        .as_ref()
        .and_then(|mail| mail.from())
        .and_then(|addresses| addresses.first());
    let email = address
        .and_then(|address| address.address())
        .unwrap_or("")
        .to_owned();
    let recipients: Vec<String> = parsed
        .as_ref()
        .and_then(|mail| mail.to())
        .map(|addresses| {
            addresses
                .iter()
                .filter_map(|address| address.address().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(ParsedMessage {
        raw_cache_key: None,
        attachments: parsed
            .as_ref()
            .map(|mail| {
                mail.attachments()
                    .enumerate()
                    .map(|(index, part)| Attachment {
                        index,
                        name: part
                            .attachment_name()
                            .unwrap_or("attachment.bin")
                            .chars()
                            .take(240)
                            .collect(),
                        size: part.contents().len(),
                        media_type: part
                            .content_type()
                            .map(|t| {
                                format!("{}/{}", t.ctype(), t.subtype().unwrap_or("octet-stream"))
                            })
                            .unwrap_or_else(|| "application/octet-stream".into()),
                    })
                    .take(200)
                    .collect()
            })
            .unwrap_or_default(),
        content_hash: mail_recovery::hash(raw),
        header_message_id: parsed
            .as_ref()
            .and_then(|mail| mail.message_id())
            .map(str::to_owned),
        uid,
        subject: parsed
            .as_ref()
            .and_then(|mail| mail.subject())
            .unwrap_or("（无主题）")
            .into(),
        sender_name: address
            .and_then(|address| address.name())
            .filter(|name| !name.is_empty())
            .unwrap_or(&email)
            .into(),
        sender_email: email,
        recipients_json: serde_json::to_string(&recipients)
            .map_err(|_| AppError::invalid("邮件收件人无效"))?,
        body_text: parsed
            .as_ref()
            .and_then(|mail| mail.body_text(0))
            .map(|text| text.to_string())
            .unwrap_or_default(),
        received_at,
        unread,
        starred,
    })
}

pub(crate) fn matches_cached(
    parsed: &ParsedMessage,
    cached: &crate::mail_store::MessageRow,
) -> bool {
    if let Some(hash) = &cached.content_hash {
        return hash == &parsed.content_hash;
    }
    !cached.body_text.trim().is_empty()
        && !cached.from.email.is_empty()
        && cached.subject == parsed.subject
        && cached.from.email == parsed.sender_email
        && cached.body_text == parsed.body_text
        && cached.received_at == parsed.received_at
        && serde_json::from_str::<Vec<String>>(&parsed.recipients_json)
            .is_ok_and(|to| to == cached.to)
}

fn sync_folder<S: Read + Write>(
    db: &SharedDb,
    plan: &SyncPlan,
    session: &mut imap::Session<S>,
    kind: &str,
    remote: &str,
    fetch: Fetcher<S>,
) -> Result<(usize, bool), AppError> {
    let (mailbox, highest) = if plan.condstore {
        crate::imap_condstore::select(session, remote, false)?
    } else {
        (
            session.select(remote).map_err(|error| {
                crate::imap_limits::network(error, &format!("无法打开 {remote} 文件夹"))
            })?,
            None,
        )
    };
    let validity = mailbox
        .uid_validity
        .filter(|uid| *uid > 0)
        .ok_or_else(|| AppError::network("IMAP 服务器未提供有效 UIDVALIDITY"))?;
    let next = mailbox
        .uid_next
        .filter(|uid| *uid > 0)
        .ok_or_else(|| AppError::network("IMAP 服务器未提供有效 UIDNEXT"))?;
    let mut expected = plan.cursors.get(kind).copied().flatten();
    let mut start = match expected {
        Some(cursor) if cursor.uid_validity == validity => cursor.last_uid.saturating_add(1),
        _ => next.saturating_sub(INITIAL_WINDOW).max(1),
    };
    let mut count = 0;
    let mut batches = 0;
    let mut more = false;
    let initial_start = start;
    let mut previous = if let Some(highest) = highest {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let saved:Option<String>=conn.query_row("SELECT highest FROM imap_modseq WHERE account_id=?1 AND folder=?2 AND generation=?3 AND uid_validity=?4",params![plan.account_id,remote,plan.generation,validity],|r|r.get(0)).optional().map_err(|_|AppError::storage("无法读取标记增量水位"))?;
        saved
            .map(|text| {
                text.parse::<u64>()
                    .map_err(|_| AppError::storage("标记增量水位损坏"))
            })
            .transpose()?
            .filter(|n| *n > 0 && *n <= highest)
    } else {
        None
    };
    let known: Vec<(String, u32, i64)> = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        ensure_current(&conn, plan)?;
        let mut stmt=conn.prepare("SELECT id,remote_uid,state_revision FROM messages WHERE account_id=?1 AND remote_folder=?2 AND remote_uid_validity=?3 AND remote_generation=?4 ORDER BY remote_uid").map_err(|_|AppError::storage("无法读取标记快照"))?;
        let result = stmt
            .query_map(
                params![plan.account_id, remote, validity, plan.generation],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| AppError::storage("无法读取标记快照"))?
            .collect::<Result<_, _>>()
            .map_err(|_| AppError::storage("邮件定位损坏"))?;
        result
    };
    loop {
        let end = if mailbox.exists == 0 {
            next.saturating_sub(1)
        } else {
            start
                .saturating_add(FETCH_UID_SPAN - 1)
                .min(next.saturating_sub(1))
        };
        let messages = if start <= end && mailbox.exists > 0 {
            let messages = fetch(session, start, end, remote, validity, &plan.cache_root)?;
            if messages.len() > FETCH_UID_SPAN as usize {
                return Err(AppError::network("服务器返回的邮件批次超出请求上限"));
            }
            let unique: std::collections::HashSet<_> =
                messages.iter().map(|message| message.uid).collect();
            if unique.len() != messages.len()
                || messages
                    .iter()
                    .any(|message| message.uid < start || message.uid > end)
            {
                return Err(AppError::network(
                    "服务器返回了请求范围外的 UID，本批次未提交",
                ));
            }
            messages
        } else {
            Vec::new()
        };
        let last_uid = end.max(
            expected
                .filter(|cursor| cursor.uid_validity == validity)
                .map_or(0, |cursor| cursor.last_uid),
        );
        let next_cursor = Cursor {
            uid_validity: validity,
            last_uid,
        };
        commit_batch(db, plan, kind, remote, expected, next_cursor, &messages)?;
        count += messages.len();
        expected = Some(next_cursor);
        batches += 1;
        if last_uid >= next.saturating_sub(1) || mailbox.exists == 0 {
            break;
        }
        if batches >= 20 {
            more = true;
            break;
        }
        start = last_uid.saturating_add(1);
    }
    let mut watermark_safe = true;
    for batch in known.chunks(100) {
        let set = batch
            .iter()
            .map(|(_, uid, _)| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let flags = if let Some(previous_value) = previous {
            if Some(previous_value) == highest {
                vec![]
            } else {
                match crate::imap_condstore::changed(
                    session,
                    &batch.iter().map(|(_, uid, _)| *uid).collect::<Vec<_>>(),
                    previous_value,
                )? {
                    Some(flags) => flags,
                    None => {
                        previous = None;
                        full_flags(session, &set)?
                    }
                }
            }
        } else {
            full_flags(session, &set)?
        };
        let mut observed = HashMap::new();
        for (uid, unread, starred) in flags {
            if !batch.iter().any(|(_, expected, _)| *expected == uid) || observed.contains_key(&uid)
            {
                return Err(AppError::network("标记响应 UID 不在请求范围内或重复"));
            }
            observed.insert(uid, (unread, starred));
        }
        // Missing FLAGS alone is insufficient evidence of expunge. Independently
        // search the exact UID set and recheck the selected namespace first.
        let missing: Vec<u32> = batch
            .iter()
            .filter(|(_, uid, _)| !observed.contains_key(uid))
            .map(|(_, uid, _)| *uid)
            .collect();
        if !missing.is_empty() {
            let set = missing
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let present = session
                .uid_search(format!("UID {set}"))
                .map_err(|e| crate::imap_limits::network(e, "无法核对消失的远端 UID"))?;
            if present.iter().any(|uid| !missing.contains(uid)) {
                return Err(AppError::network("UID 存在性响应越界，未更新本机"));
            }
            let boundary = session
                .examine(remote)
                .map_err(|e| crate::imap_limits::network(e, "无法复核删除命名空间"))?;
            if boundary.uid_validity != Some(validity) {
                return Err(AppError::conflict("删除核对期间 UIDVALIDITY 已变化"));
            }
            let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            let tx = conn
                .transaction()
                .map_err(|_| AppError::storage("无法保存远端存在性"))?;
            ensure_current(&tx, plan)?;
            for (id, uid, revision) in batch
                .iter()
                .filter(|(_, uid, _)| missing.contains(uid) && !present.contains(uid))
            {
                let changed=tx.execute("UPDATE messages SET remote_missing=1,state_revision=state_revision+1 WHERE account_id=?1 AND id=?2 AND state_revision=?3 AND remote_uid=?4 AND remote_uid_validity=?5 AND remote_generation=?6 AND remote_missing=0 AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=messages.id AND r.state NOT IN ('restored','local'))",params![plan.account_id,id,revision,uid,validity,plan.generation]).map_err(|_|AppError::storage("无法保存远端存在性"))?;
                if changed == 1 {
                    tx.execute("UPDATE message_changes SET state='conflict',error='远端原 UID 已不存在，本机修改保留，请核对定位。' WHERE account_id=?1 AND message_id=?2 AND state IN ('queued','failed')",params![plan.account_id,id]).map_err(|_|AppError::storage("无法保留删除冲突"))?;
                }
            }
            tx.commit()
                .map_err(|_| AppError::storage("远端存在性提交失败"))?;
        }
        let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let tx = conn
            .transaction()
            .map_err(|_| AppError::storage("无法保存标记"))?;
        ensure_current(&tx, plan)?;
        for (id, uid, revision) in batch {
            if plan.condstore {
                let applicable:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m WHERE m.id=?1 AND m.state_revision=?2 AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=m.id AND r.state!='restored'))",params![id,revision],|r|r.get(0)).map_err(|_|AppError::storage("无法核对标记增量版本"))?;
                watermark_safe &= applicable;
            }
            if let Some((unread, starred)) = observed.get(uid) {
                message_changes::observe_flags(&tx, id, *revision, *unread, *starred)?;
                tx.execute("UPDATE messages SET remote_missing=0,state_revision=state_revision+1 WHERE id=?1 AND account_id=?2 AND remote_missing=1 AND remote_uid=?3 AND remote_uid_validity=?4 AND remote_generation=?5",params![id,plan.account_id,uid,validity,plan.generation]).map_err(|_|AppError::storage("无法恢复远端存在状态"))?;
            }
        }
        tx.commit()
            .map_err(|_| AppError::storage("服务器标记提交失败"))?;
    }
    {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        ensure_current(&conn, plan)?;
        let first = initial_start;
        conn.execute("INSERT INTO imap_history(account_id,folder,generation,uid_validity,before_uid) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(account_id,folder) DO UPDATE SET generation=excluded.generation,uid_validity=excluded.uid_validity,before_uid=excluded.before_uid WHERE imap_history.generation!=excluded.generation OR imap_history.uid_validity!=excluded.uid_validity",params![plan.account_id,remote,plan.generation,validity,first]).map_err(|_|AppError::storage("无法保存历史边界"))?;
    }
    if plan.settings.history_enabled {
        let before: u32 = {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            conn.query_row("SELECT before_uid FROM imap_history WHERE account_id=?1 AND folder=?2 AND generation=?3 AND uid_validity=?4",params![plan.account_id,remote,plan.generation,validity],|r|r.get(0)).map_err(|_|AppError::storage("无法读取历史边界"))?
        };
        if before > 1 {
            let end = before - 1;
            let start = end.saturating_sub(FETCH_UID_SPAN - 1).max(1);
            let messages = fetch(session, start, end, remote, validity, &plan.cache_root)?;
            if messages.len() > FETCH_UID_SPAN as usize
                || messages.iter().any(|m| m.uid < start || m.uid > end)
            {
                return Err(AppError::network("历史邮件响应越界，未推进回填"));
            }
            commit_batch(
                db,
                plan,
                kind,
                remote,
                expected,
                expected.ok_or_else(|| AppError::conflict("历史回填缺少已提交游标"))?,
                &messages,
            )?;
            count += messages.len();
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            ensure_current(&conn, plan)?;
            conn.execute("UPDATE imap_history SET before_uid=?1 WHERE account_id=?2 AND folder=?3 AND generation=?4 AND uid_validity=?5 AND before_uid=?6",params![start,plan.account_id,remote,plan.generation,validity,before]).map_err(|_|AppError::storage("无法保存历史进度"))?;
            more |= start > 1;
        }
    }
    if plan.condstore {
        let final_highest = if highest.is_some() {
            let (mailbox, seq) = crate::imap_condstore::select(session, remote, true)?;
            if mailbox.uid_validity != Some(validity) {
                return Err(AppError::conflict("标记增量核对期间 UIDVALIDITY 改变"));
            }
            seq
        } else {
            None
        };
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        ensure_current(&conn, plan)?;
        if watermark_safe
            && highest.is_some_and(|initial| final_highest.is_some_and(|now| now >= initial))
        {
            conn.execute("INSERT INTO imap_modseq(account_id,folder,generation,uid_validity,highest) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(account_id,folder) DO UPDATE SET generation=excluded.generation,uid_validity=excluded.uid_validity,highest=excluded.highest",params![plan.account_id,remote,plan.generation,validity,highest.unwrap().to_string()]).map_err(|_|AppError::storage("无法保存标记增量水位"))?;
        } else {
            conn.execute(
                "DELETE FROM imap_modseq WHERE account_id=?1 AND folder=?2",
                params![plan.account_id, remote],
            )
            .map_err(|_| AppError::storage("无法重置标记增量水位"))?;
            if previous.is_some() && watermark_safe {
                return Err(AppError::network(
                    "MODSEQ 回退或失效，下次完整读取标记；本机修改保留",
                ));
            }
        }
    }
    Ok((count, more))
}
fn full_flags<S: Read + Write>(
    session: &mut imap::Session<S>,
    set: &str,
) -> Result<Vec<(u32, bool, bool)>, AppError> {
    let flags = session
        .uid_fetch(set, "(UID FLAGS)")
        .map_err(|e| crate::imap_limits::network(e, "无法刷新已有邮件标记"))?;
    flags
        .iter()
        .map(|m| {
            Ok((
                m.uid.ok_or_else(|| AppError::network("标记响应缺少 UID"))?,
                !m.flags().contains(&imap::types::Flag::Seen),
                m.flags().contains(&imap::types::Flag::Flagged),
            ))
        })
        .collect()
}

fn commit_batch(
    db: &SharedDb,
    plan: &SyncPlan,
    kind: &str,
    remote: &str,
    expected: Option<Cursor>,
    cursor: Cursor,
    messages: &[ParsedMessage],
) -> Result<(), AppError> {
    crate::sync_control::check()?;
    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法开始邮件保存"))?;
    ensure_current(&tx, plan)?;
    if read_cursor(&tx, &plan.account_id, kind)? != expected {
        return Err(AppError::conflict("同步游标已改变，旧同步结果未写入。"));
    }
    if expected.is_none_or(|previous| previous.uid_validity != cursor.uid_validity) {
        tx.execute("UPDATE message_changes SET state='conflict',error='UIDVALIDITY 已改变，保留本机修改等待核对。' WHERE message_id IN (SELECT id FROM messages WHERE account_id=?1 AND remote_folder=?2 AND remote_uid_validity!=?3) AND state NOT IN ('unknown','local')",
          params![plan.account_id,remote,cursor.uid_validity]).map_err(|_|AppError::storage("无法保留同步冲突"))?;
        tx.execute("DELETE FROM messages WHERE account_id=?1 AND remote_folder=?2 AND remote_uid_validity!=?3 AND original_remote_id IS NULL AND NOT EXISTS(SELECT 1 FROM message_changes c WHERE c.message_id=messages.id) AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=messages.id AND r.state!='restored')",
          params![plan.account_id, remote,cursor.uid_validity])
          .map_err(|_| AppError::storage("无法更新已失效邮件"))?;
    }
    let folder_id = format!("{}:folder:{remote}", plan.account_id);
    for message in messages {
        let mut remote_id = format!("imap:{kind}:{}:{}", cursor.uid_validity, message.uid);
        let protected: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN message_recovery r ON r.message_id=m.id WHERE m.account_id=?1 AND m.remote_id=?2 AND r.state!='restored')",params![plan.account_id,remote_id],|r|r.get(0)).map_err(|_|AppError::storage("无法核对缓存保护状态"))?;
        if protected {
            continue;
        }
        let collision: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT id,remote_generation FROM messages WHERE account_id=?1 AND remote_id=?2",
                params![plan.account_id, remote_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| AppError::storage("无法核对历史定位"))?;
        if let Some((old_id, origin)) =
            collision.filter(|(_, origin)| origin.as_deref() != Some(&plan.generation))
        {
            let _ = origin;
            tx.execute("UPDATE messages SET original_remote_id=COALESCE(original_remote_id,remote_id),remote_id=?1,remote_missing=1,state_revision=state_revision+1 WHERE account_id=?2 AND id=?3",params![format!("retired:{}",uuid::Uuid::new_v4()),plan.account_id,old_id]).map_err(|_|AppError::storage("无法保护旧账号缓存"))?;
            tx.execute("UPDATE message_changes SET state='conflict',error='连接代次已改变，原邮件和修改保留，需明确核对新定位。' WHERE account_id=?1 AND message_id=?2 AND state IN ('queued','failed')",params![plan.account_id,old_id]).map_err(|_|AppError::storage("无法保留旧账号修改"))?;
        }
        let existing = {
            let mut stmt=tx.prepare("SELECT remote_id FROM messages WHERE account_id=?1 AND remote_folder=?2 AND remote_uid_validity=?3 AND remote_uid=?4 AND remote_generation=?5 AND locator_verified=1 LIMIT 2").map_err(|_|AppError::storage("无法读取既有定位"))?;
            let rows = stmt
                .query_map(
                    params![
                        plan.account_id,
                        remote,
                        cursor.uid_validity,
                        message.uid,
                        plan.generation
                    ],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|_| AppError::storage("无法读取既有定位"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| AppError::storage("既有定位损坏"))?;
            rows
        };
        if existing.len() > 1 {
            return Err(AppError::conflict(
                "同一远端 UID 有多份本机缓存，未选择或覆盖其中一份",
            ));
        }
        if let Some(existing) = existing.into_iter().next() {
            remote_id = existing;
        }
        let generated = format!("{}:{remote_id}", plan.account_id);
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1)",
                [&generated],
                |r| r.get(0),
            )
            .map_err(|_| AppError::storage("无法核对本机 ID"))?;
        let id = if exists {
            format!("{}:{}", plan.account_id, uuid::Uuid::new_v4())
        } else {
            generated
        };
        let protected:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN message_recovery r ON r.message_id=m.id WHERE m.account_id=?1 AND m.remote_id=?2 AND r.state!='restored')",params![plan.account_id,remote_id],|r|r.get(0)).map_err(|_|AppError::storage("无法核对缓存保护状态"))?;
        if protected {
            continue;
        }
        tx.execute("INSERT INTO messages (id,account_id,remote_id,thread_id,subject,sender_name,sender_email,recipients_json,body_text,received_at,unread,starred,remote_folder,remote_uid_validity,remote_uid,remote_generation,content_hash,header_message_id,locator_verified,content_hash_verified)
          VALUES (?1,?2,?3,?1,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,1,1)
          ON CONFLICT(account_id,remote_id) DO UPDATE SET subject=excluded.subject,sender_name=excluded.sender_name,sender_email=excluded.sender_email,
          recipients_json=excluded.recipients_json,body_text=excluded.body_text,received_at=excluded.received_at,
          unread=COALESCE((SELECT value FROM message_changes WHERE message_id=messages.id AND field='unread'),excluded.unread),
          starred=COALESCE((SELECT value FROM message_changes WHERE message_id=messages.id AND field='starred'),excluded.starred),
          remote_folder=excluded.remote_folder,remote_uid_validity=excluded.remote_uid_validity,remote_uid=excluded.remote_uid,remote_generation=excluded.remote_generation,content_hash=excluded.content_hash,header_message_id=excluded.header_message_id,locator_verified=1,content_hash_verified=1,state_revision=messages.state_revision+1",
          params![id, plan.account_id, remote_id, message.subject, message.sender_name, message.sender_email, message.recipients_json, message.body_text, message.received_at, message.unread, message.starred,remote,cursor.uid_validity,message.uid,plan.generation,message.content_hash,message.header_message_id])
          .map_err(|_| AppError::storage("邮件批次未能保存"))?;
        let id: String = tx
            .query_row(
                "SELECT id FROM messages WHERE account_id=?1 AND remote_id=?2",
                params![plan.account_id, remote_id],
                |r| r.get(0),
            )
            .map_err(|_| AppError::storage("无法读取稳定邮件 ID"))?;
        tx.execute("UPDATE messages SET raw_cache_key=COALESCE(?1,raw_cache_key),attachments_json=?2,remote_missing=0 WHERE account_id=?3 AND id=?4", params![message.raw_cache_key, serde_json::to_string(&message.attachments).map_err(|_| AppError::storage("无法保存附件信息"))?, plan.account_id,id]).map_err(|_| AppError::storage("无法保存原文缓存信息"))?;
        tx.execute(
            "INSERT OR IGNORE INTO message_folders VALUES (?1,?2,?3)",
            params![plan.account_id, id, folder_id],
        )
        .map_err(|_| AppError::storage("邮件文件夹关联未能保存"))?;
    }
    tx.execute("INSERT INTO sync_states (account_id,scope,cursor,updated_at) VALUES (?1,?2,?3,?4)
      ON CONFLICT(account_id,scope) DO UPDATE SET cursor=excluded.cursor,updated_at=excluded.updated_at",
      params![plan.account_id, format!("imap:{kind}"), serde_json::to_string(&cursor).map_err(|_| AppError::invalid("同步游标无效"))?, crate::now_utc()])
      .map_err(|_| AppError::storage("同步游标未能保存"))?;
    tx.commit()
        .map_err(|_| AppError::storage("邮件批次未完成提交"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor as IoCursor, sync::mpsc, thread, time::Duration};
    // Transaction / concurrency fixtures supply already scripted complete FETCH;
    // production size planning and partial parsing have their own protocol tests.
    pub(super) fn fixture_fetch<S: Read + Write>(
        s: &mut imap::Session<S>,
        start: u32,
        end: u32,
        _: &str,
        _: u32,
        _: &std::path::Path,
    ) -> Result<Vec<ParsedMessage>, AppError> {
        s.uid_fetch(
            format!("{start}:{end}"),
            "(UID FLAGS INTERNALDATE BODY.PEEK[])",
        )
        .map_err(|e| crate::imap_limits::network(e, "测试 FETCH 失败"))?
        .iter()
        .map(parse_message)
        .collect()
    }

    #[derive(Debug)]
    struct MockStream {
        reads: IoCursor<Vec<u8>>,
    }
    impl Read for MockStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads.read(buffer)
        }
    }
    impl Write for MockStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    pub(super) fn db() -> SharedDb {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO accounts (id,provider,email,display_name,profile_json,created_at) VALUES ('acc','imap','me@example.test','Test',?1,'2026-11-11T00:00:00Z')",
          [serde_json::json!({"imapHost":"127.0.0.1","imapPort":1,"authUser":"me@example.test","status":"offline"}).to_string()]).unwrap();
        Arc::new(Mutex::new(conn))
    }
    pub(super) fn plan(db: &SharedDb) -> SyncPlan {
        prepare(&db.lock().unwrap(), "acc").unwrap()
    }
    fn prefix(validity: u32, next: u32) -> String {
        format!("* OK IMAP4rev1 ready\r\na1 OK login\r\n* LIST (\\HasNoChildren) \"/\" INBOX\r\na2 OK list\r\n* 1 EXISTS\r\n* OK [UIDVALIDITY {validity}] valid\r\n* OK [UIDNEXT {next}] next\r\na3 OK selected\r\n")
    }
    pub(super) fn fetch(uid: u32, tag: u32, subject: &str) -> String {
        let body = format!("Subject: {subject}\r\nFrom: Alice <alice@example.test>\r\nTo: me@example.test\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from Alice\r\n");
        format!("* 1 FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"10-Nov-2026 03:24:00 +0000\" BODY[] {{{}}}\r\n{body})\r\na{tag} OK fetched\r\n", body.len())
    }
    fn run(db: &SharedDb, script: String) -> Result<SyncOutcome, AppError> {
        let plan = plan(db);
        let client = imap::Client::new(MockStream {
            reads: IoCursor::new(script.into_bytes()),
        });
        let session = client.login("me@example.test", "fixture-secret").unwrap();
        sync_with_session(db, &plan, session)
    }
    pub(super) fn first(db: &SharedDb) {
        run(
            db,
            format!(
                "{}{}* BYE bye\r\na5 OK logout\r\n",
                prefix(1000, 1002),
                fetch(1001, 4, "=?UTF-8?B?5L2g5aW9?=")
            ),
        )
        .unwrap();
    }

    #[test]
    fn streamed_oversize_fetch_keeps_the_previous_uidvalidity_cursor_cache_and_local_edit() {
        use crate::imap_writeback::tests::{session, step};
        let db = db();
        first(&db);
        let id: String = db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM messages", [], |r| r.get(0))
            .unwrap();
        message_changes::update(
            &mut db.lock().unwrap(),
            "acc",
            &id,
            &message_changes::Patch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        db.lock()
            .unwrap()
            .execute("UPDATE message_changes SET state='conflict'", [])
            .unwrap();
        let plan = plan(&db);
        let unlocked = db.clone();
        let (s, trace, _) = session(vec![
            step("LIST", "* LIST () \"/\" INBOX\r\n{tag} OK list\r\n"),
            step("SELECT", "* 1 EXISTS\r\n* OK [UIDVALIDITY 2000] changed\r\n* OK [UIDNEXT 2] next\r\n{tag} OK selected\r\n"),
            step("UID FETCH", "* 1 FETCH (UID 1 BODY[] {999999999999}\r\n"),
        ], Some(Box::new(move |_| { assert!(unlocked.try_lock().is_ok()); })));
        let error = sync_with_session_trace(&db, &plan, s, Some(&trace)).unwrap_err();
        assert!(error.message.contains("安全限额"));
        let conn = db.lock().unwrap();
        assert_eq!(
            read_cursor(&conn, "acc", "inbox").unwrap(),
            Some(Cursor {
                uid_validity: 1000,
                last_uid: 1001
            })
        );
        let m = crate::mail_store::message_by_id(&conn, "acc", &id).unwrap();
        assert!(m.starred);
        assert_eq!(m.subject, "你好");
        assert_eq!(m.remote_id.as_deref(), Some("imap:inbox:1000:1001"));
    }

    #[test]
    fn first_and_incremental_sync_preserve_decoded_text_flags_and_account_scope() {
        let db = db();
        first(&db);
        {
            let conn = db.lock().unwrap();
            let (subject, sender, unread, body): (String, String, i64, String) = conn
                .query_row(
                    "SELECT subject,sender_name,unread,body_text FROM messages",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            assert_eq!(subject, "你好");
            assert_eq!(sender, "Alice");
            assert_eq!(unread, 0);
            assert!(body.contains("Hello from Alice"));
        }
        run(
            &db,
            format!(
                "{}{}* BYE bye\r\na5 OK logout\r\n",
                prefix(1000, 1003),
                fetch(1002, 4, "Second")
            )
            .replace(
                "* BYE bye\r\na5 OK logout",
                "* 1 FETCH (UID 1001 FLAGS (\\Seen))\r\na5 OK flags\r\n* BYE bye\r\na6 OK logout",
            ),
        )
        .unwrap();
        let conn = db.lock().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id='acc'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            read_cursor(&conn, "acc", "inbox").unwrap(),
            Some(Cursor {
                uid_validity: 1000,
                last_uid: 1002
            })
        );
    }

    #[test]
    fn changed_uidvalidity_purges_only_after_a_successful_fetch() {
        let db = db();
        first(&db);
        let error = run(&db, format!("{}a4 NO fetch failed\r\n", prefix(2000, 5002))).unwrap_err();
        assert_eq!(error.code, "network");
        {
            let conn = db.lock().unwrap();
            assert_eq!(
                read_cursor(&conn, "acc", "inbox")
                    .unwrap()
                    .unwrap()
                    .uid_validity,
                1000
            );
            assert_eq!(
                conn.query_row("SELECT subject FROM messages", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "你好"
            );
        }
        run(
            &db,
            format!(
                "{}{}* BYE bye\r\na5 OK logout\r\n",
                prefix(2000, 5002),
                fetch(5001, 4, "New")
            ),
        )
        .unwrap();
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("SELECT subject FROM messages", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "New"
        );
    }

    #[test]
    fn sql_failure_rolls_back_purge_messages_and_cursor_together() {
        let db = db();
        first(&db);
        db.lock().unwrap().execute_batch("CREATE TRIGGER fail_cursor BEFORE INSERT ON sync_states BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        assert_eq!(
            run(
                &db,
                format!("{}{}", prefix(2000, 5002), fetch(5001, 4, "New"))
            )
            .unwrap_err()
            .code,
            "storage"
        );
        let conn = db.lock().unwrap();
        assert_eq!(
            read_cursor(&conn, "acc", "inbox")
                .unwrap()
                .unwrap()
                .uid_validity,
            1000
        );
        assert_eq!(
            conn.query_row("SELECT subject FROM messages", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "你好"
        );
    }

    #[derive(Debug)]
    struct BlockingStream {
        prefix: IoCursor<Vec<u8>>,
        suffix: IoCursor<Vec<u8>>,
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }
    impl Read for BlockingStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.prefix.read(buffer)?;
            if read > 0 {
                return Ok(read);
            }
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                self.release
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::TimedOut, "fixture gate")
                    })?;
            }
            self.suffix.read(buffer)
        }
    }
    impl Write for BlockingStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn blocked_network_fetch_does_not_block_draft_save_or_workspace_reads() {
        let db = db();
        let prepared = plan(&db);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let stream = BlockingStream {
            prefix: IoCursor::new(prefix(1000, 1002).into_bytes()),
            suffix: IoCursor::new(
                format!("{}* BYE bye\r\na5 OK logout\r\n", fetch(1001, 4, "New")).into_bytes(),
            ),
            entered: Some(entered_tx),
            release: release_rx,
        };
        let worker_db = db.clone();
        let handle = thread::spawn(move || {
            let session = imap::Client::new(stream)
                .login("me@example.test", "fixture")
                .unwrap();
            sync_with_session(&worker_db, &prepared, session)
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let saved = match db.try_lock() {
            Ok(mut conn) => {
                crate::mail_store::load_workspace(&conn).unwrap();
                crate::drafts::save(
                    &mut conn,
                    &crate::drafts::Draft {
                        id: "draft-while-syncing".into(),
                        account_id: "acc".into(),
                        to: String::new(),
                        subject: "Keep".into(),
                        body_text: "Saved while fetch was blocked".into(),
                        reply_to_id: None,
                        updated_at: crate::now_utc(),
                        revision: 0,
                    },
                    "acc",
                )
                .map(|_| ())
            }
            Err(_) => Err(AppError::storage("network still owns DB lock")),
        };
        release_tx.send(()).unwrap();
        let result = handle.join().unwrap();
        assert!(
            saved.is_ok(),
            "draft saving must work before releasing the network gate"
        );
        assert!(result.is_ok());
        assert_eq!(crate::drafts::list(&db.lock().unwrap()).unwrap().len(), 1);
    }

    #[test]
    fn changed_or_recreated_accounts_and_changed_cursors_reject_stale_commits() {
        let db = db();
        first(&db);
        let prepared = plan(&db);
        let cursor = prepared.cursors["inbox"];
        let next = Cursor {
            uid_validity: 1000,
            last_uid: 1002,
        };
        commit_batch(&db, &prepared, "inbox", "INBOX", cursor, next, &[]).unwrap();
        assert_eq!(
            commit_batch(&db, &prepared, "inbox", "INBOX", cursor, next, &[])
                .unwrap_err()
                .code,
            "conflict"
        );
        db.lock()
            .unwrap()
            .execute(
                "UPDATE accounts SET email='changed@example.test' WHERE id='acc'",
                [],
            )
            .unwrap();
        assert_eq!(
            commit_batch(&db, &prepared, "inbox", "INBOX", Some(next), next, &[])
                .unwrap_err()
                .code,
            "conflict"
        );
        let prepared = plan(&db);
        {
            let conn = db.lock().unwrap();
            let profile: String = conn
                .query_row(
                    "SELECT profile_json FROM accounts WHERE id='acc'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            conn.execute("DELETE FROM accounts WHERE id='acc'", [])
                .unwrap();
            conn.execute("INSERT INTO accounts (id,provider,email,display_name,profile_json,created_at) VALUES ('acc','imap','changed@example.test','Test',?1,'2026-11-11T00:00:00Z')",[profile]).unwrap();
        }
        assert_eq!(
            commit_batch(&db, &prepared, "inbox", "INBOX", None, next, &[])
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn a_later_batch_failure_keeps_the_last_committed_cursor() {
        let db = db();
        first(&db);
        assert!(run(
            &db,
            format!(
                "{}{}a5 NO later batch failed\r\n",
                prefix(1000, 1252),
                fetch(1002, 4, "Committed")
            )
        )
        .is_err());
        let conn = db.lock().unwrap();
        assert_eq!(
            read_cursor(&conn, "acc", "inbox")
                .unwrap()
                .unwrap()
                .last_uid,
            1101
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn incomplete_or_out_of_range_responses_never_advance_the_cursor() {
        let db = db();
        first(&db);
        assert!(run(
            &db,
            format!(
                "{}* 1 FETCH (UID 1002 FLAGS ())\r\na4 OK fetched\r\n",
                prefix(1000, 1003)
            )
        )
        .is_err());
        assert!(run(
            &db,
            format!("{}{}", prefix(1000, 1003), fetch(1001, 4, "Wrong range"))
        )
        .is_err());
        assert_eq!(
            read_cursor(&db.lock().unwrap(), "acc", "inbox")
                .unwrap()
                .unwrap()
                .last_uid,
            1001
        );
    }

    #[test]
    fn sent_network_failure_is_reported_without_hiding_committed_inbox() {
        let db = db();
        let script = prefix(1000, 1002).replace(
            "a2 OK list",
            "* LIST (\\HasNoChildren) \"/\" \"Sent Items\"\r\na2 OK list",
        );
        let outcome = run(
            &db,
            format!(
                "{script}{}a5 NO cannot select sent\r\n* BYE bye\r\na6 OK logout\r\n",
                fetch(1001, 4, "Inbox")
            ),
        )
        .unwrap();
        assert_eq!(outcome.fetched_messages, 1);
        assert_eq!(outcome.warnings.len(), 1);
    }

    #[test]
    fn folder_mapping_and_cursor_validation_are_explicit() {
        for (name, kind) in [
            ("INBOX", "inbox"),
            ("Sent Items", "sent"),
            ("已发送", "sent"),
            ("Drafts", "drafts"),
            ("Archive/2026", "archive"),
            ("Deleted Items", "trash"),
            ("Projects", "other"),
        ] {
            assert_eq!(folder_kind(name), kind);
        }
        let db = db();
        db.lock().unwrap().execute("INSERT INTO sync_states VALUES ('acc','imap:inbox','{\"uidValidity\":1,\"lastUid\":4294967296}','2026-11-11T00:00:00Z')",[]).unwrap();
        assert!(prepare(&db.lock().unwrap(), "acc").is_err());
    }

    #[test]
    fn two_scheduled_imap_fetches_share_the_db_without_blocking_drafts() {
        let db = db();
        db.lock().unwrap().execute("INSERT INTO accounts (id,provider,email,display_name,profile_json,created_at) SELECT 'acc2',provider,'other@example.test',display_name,profile_json,created_at FROM accounts WHERE id='acc'",[]).unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_a, rx_a) = mpsc::channel();
        let (release_b, rx_b) = mpsc::channel();
        let gates = Mutex::new(HashMap::from([
            ("acc".to_string(), rx_a),
            ("acc2".to_string(), rx_b),
        ]));
        let worker_db = db.clone();
        let scheduler = crate::sync_scheduler::SyncScheduler::new(move |id| {
            let prepared = prepare(&worker_db.lock().unwrap(), id)?;
            let stream = BlockingStream {
                prefix: IoCursor::new(prefix(1000, 1002).into_bytes()),
                suffix: IoCursor::new(
                    format!(
                        "{}* BYE bye\r\na5 OK logout\r\n",
                        fetch(1001, 4, "Shared DB")
                    )
                    .into_bytes(),
                ),
                entered: Some(entered_tx.clone()),
                release: gates.lock().unwrap().remove(id).unwrap(),
            };
            let session = imap::Client::new(stream)
                .login("me@example.test", "fixture")
                .unwrap();
            sync_with_session(&worker_db, &prepared, session)
        })
        .unwrap();
        let a = scheduler.ticket("acc".into()).unwrap();
        let b = scheduler.ticket("acc2".into()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let draft_saved = match db.try_lock() {
            Ok(mut conn) => crate::drafts::save(
                &mut conn,
                &crate::drafts::Draft {
                    id: "draft-multi-sync".into(),
                    account_id: "acc2".into(),
                    to: String::new(),
                    subject: String::new(),
                    body_text: "Both fetches blocked".into(),
                    reply_to_id: None,
                    updated_at: crate::now_utc(),
                    revision: 0,
                },
                "acc2",
            )
            .is_ok(),
            Err(_) => false,
        };
        release_a.send(()).unwrap();
        release_b.send(()).unwrap();
        a.wait().unwrap();
        b.wait().unwrap();
        assert!(draft_saved);
        let conn = db.lock().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(DISTINCT account_id) FROM messages",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }

    #[test]
    fn cached_flags_refresh_without_new_mail_and_preserve_local_edits_during_fetch() {
        let db = db();
        first(&db);
        run(&db,format!("{}* 1 FETCH (UID 1001 FLAGS (\\Flagged))\r\na4 OK flags\r\n* BYE bye\r\na5 OK logout\r\n",prefix(1000,1002))).unwrap();
        {
            let conn = db.lock().unwrap();
            let saved =
                crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001").unwrap();
            assert!(saved.unread && saved.starred);
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let stream=BlockingStream{prefix:IoCursor::new(prefix(1000,1002).into_bytes()),suffix:IoCursor::new(b"* 1 FETCH (UID 1001 FLAGS (\\Seen))\r\na4 OK flags\r\n* BYE bye\r\na5 OK logout\r\n".to_vec()),entered:Some(entered_tx),release:release_rx};
        let other = db.clone();
        let handle = thread::spawn(move || {
            let plan = plan(&other);
            let session = imap::Client::new(stream)
                .login("me@example.test", "fixture-secret")
                .unwrap();
            sync_with_session(&other, &plan, session)
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        {
            let mut conn = db.try_lock().expect("FLAGS must not hold DB lock");
            message_changes::update(
                &mut conn,
                "acc",
                "acc:imap:inbox:1000:1001",
                &message_changes::Patch {
                    unread: Some(true),
                    starred: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        release_tx.send(()).unwrap();
        handle.join().unwrap().unwrap();
        let conn = db.lock().unwrap();
        let saved =
            crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001").unwrap();
        assert!(saved.unread && saved.starred);
        assert_eq!(saved.changes.len(), 2);
    }

    #[test]
    fn uidvalidity_purge_retains_local_intents_and_moved_messages_keep_stable_ids() {
        let db = db();
        first(&db);
        let plan = plan(&db);
        {
            let mut conn = db.lock().unwrap();
            message_changes::update(
                &mut conn,
                "acc",
                "acc:imap:inbox:1000:1001",
                &message_changes::Patch {
                    starred: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let old = plan.cursors["inbox"];
        commit_batch(
            &db,
            &plan,
            "inbox",
            "INBOX",
            old,
            Cursor {
                uid_validity: 2000,
                last_uid: 1001,
            },
            &[],
        )
        .unwrap();
        let conn = db.lock().unwrap();
        let saved =
            crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001").unwrap();
        assert!(saved.starred);
        assert_eq!(saved.changes[0].state, "conflict");
        drop(conn);
        // A previously confirmed MOVE may have a stable id different from the generated remote id.
        {
            let conn = db.lock().unwrap();
            conn.execute("UPDATE messages SET remote_id='imap:inbox:2000:9' WHERE id='acc:imap:inbox:1000:1001'",[]).unwrap();
        }
        let message = ParsedMessage {
            raw_cache_key: None,
            attachments: vec![],
            content_hash: String::new(),
            header_message_id: None,
            uid: 9,
            subject: "Moved".into(),
            sender_name: "Sender".into(),
            sender_email: "sender@example.test".into(),
            recipients_json: "[]".into(),
            body_text: "Body".into(),
            received_at: crate::now_utc(),
            unread: false,
            starred: false,
        };
        commit_batch(
            &db,
            &plan,
            "inbox",
            "INBOX",
            Some(Cursor {
                uid_validity: 2000,
                last_uid: 1001,
            }),
            Cursor {
                uid_validity: 2000,
                last_uid: 1002,
            },
            &[message],
        )
        .unwrap();
        let conn = db.lock().unwrap();
        let saved =
            crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001").unwrap();
        assert!(saved.starred);
        assert_eq!(saved.subject, "Moved");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn recovery_review_protects_cache_from_body_flags_and_uidvalidity_purge() {
        let db = db();
        first(&db);
        let id = "acc:imap:inbox:1000:1001";
        let original_revision = {
            let mut conn = db.lock().unwrap();
            conn.execute("UPDATE messages SET remote_generation=NULL", [])
                .unwrap();
            let revision = crate::mail_store::message_by_id(&conn, "acc", id)
                .unwrap()
                .state_revision;
            crate::mail_recovery::request(
                &mut conn,
                "acc",
                id,
                revision,
                crate::mail_recovery::Action::Check,
            )
            .unwrap()
            .state_revision
        };
        let plan = plan(&db);
        let old = plan.cursors["inbox"];
        let body = ParsedMessage {
            raw_cache_key: None,
            attachments: vec![],
            uid: 1001,
            subject: "Do not replace".into(),
            sender_name: "Sender".into(),
            sender_email: "sender@example.test".into(),
            recipients_json: "[]".into(),
            body_text: "Different body".into(),
            received_at: crate::now_utc(),
            unread: false,
            starred: true,
            content_hash: "different".into(),
            header_message_id: None,
        };
        commit_batch(&db, &plan, "inbox", "INBOX", old, old.unwrap(), &[body]).unwrap();
        {
            let conn = db.lock().unwrap();
            message_changes::observe_flags(&conn, id, original_revision, false, true).unwrap();
            let message = crate::mail_store::message_by_id(&conn, "acc", id).unwrap();
            assert_eq!(message.subject, "你好");
            assert!(!message.starred);
            assert_eq!(message.state_revision, original_revision);
        }
        commit_batch(
            &db,
            &plan,
            "inbox",
            "INBOX",
            old,
            Cursor {
                uid_validity: 2000,
                last_uid: 1,
            },
            &[],
        )
        .unwrap();
        let conn = db.lock().unwrap();
        assert_eq!(
            crate::mail_store::message_by_id(&conn, "acc", id)
                .unwrap()
                .subject,
            "你好"
        );
    }

    #[test]
    fn an_empty_new_uidvalidity_commits_the_empty_state_without_fetching() {
        let db = db();
        first(&db);
        let script = prefix(2000, 5002).replace("* 1 EXISTS", "* 0 EXISTS");
        let outcome = run(&db, format!("{script}* BYE bye\r\na4 OK logout\r\n")).unwrap();
        assert_eq!(outcome.fetched_messages, 0);
        let conn = db.lock().unwrap();
        assert_eq!(
            read_cursor(&conn, "acc", "inbox").unwrap(),
            Some(Cursor {
                uid_validity: 2000,
                last_uid: 5001
            })
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
