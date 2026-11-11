//! Explicit cache re-download; restores raw bytes without upgrading weak historical evidence.
use crate::{
    error::AppError,
    mail_store,
    mail_sync::{self, SharedDb},
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::io::{Read, Write};
#[cfg(test)]
#[path = "imap_body_tests.rs"]
mod tests;
#[derive(Serialize, Debug)]
pub(crate) struct Status {
    pub state: String,
    pub error: Option<String>,
}
fn storage() -> AppError {
    AppError::storage("原文下载请求不可用")
}
pub(crate) fn status(conn: &rusqlite::Connection, id: &str) -> Result<Option<Status>, AppError> {
    conn.query_row(
        "SELECT state,error FROM imap_body_requests WHERE message_id=?1",
        [id],
        |r| {
            Ok(Status {
                state: r.get(0)?,
                error: r.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|_| storage())
}
struct Snapshot {
    account: String,
    id: String,
    generation: String,
    revision: i64,
    folder: String,
    validity: u32,
    uid: u32,
    cached: mail_store::MessageRow,
    root: std::path::PathBuf,
}
fn snapshot(conn: &rusqlite::Connection, account: &str, id: &str) -> Result<Snapshot, AppError> {
    let cached = mail_store::message_by_id(conn, account, id)?;
    let (folder,validity,uid,origin,current):(Option<String>,Option<u32>,Option<u32>,Option<String>,String)=conn.query_row("SELECT m.remote_folder,m.remote_uid_validity,m.remote_uid,m.remote_generation,a.sync_generation FROM messages m JOIN accounts a ON a.id=m.account_id WHERE m.account_id=?1 AND m.id=?2 AND a.provider='imap'",params![account,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|AppError::invalid("请选择真实 IMAP 邮件"))?;
    let parts: Vec<_> = cached
        .remote_id
        .as_deref()
        .unwrap_or("")
        .split(':')
        .collect();
    let active = cached
        .recovery
        .as_ref()
        .is_some_and(|r| !matches!(r.state.as_str(), "available" | "restored" | "local"));
    if cached.remote_missing
        || !cached.locator_verified
        || active
        || origin.as_deref() != Some(&current)
        || validity.is_none_or(|n| n == 0)
        || uid.is_none_or(|n| n == 0)
        || parts.len() != 4
        || parts[0] != "imap"
        || !crate::imap_settings::valid_scope(parts[1], folder.as_deref())
        || parts[2].parse::<u32>().ok() != validity
        || parts[3].parse::<u32>().ok() != uid
    {
        return Err(AppError::conflict(
            "原定位需要先核对，未对未知 UID 下载或绑定",
        ));
    }
    let folder = folder.ok_or_else(|| AppError::conflict("原文件夹缺失"))?;
    Ok(Snapshot {
        account: account.into(),
        id: id.into(),
        generation: current,
        revision: cached.state_revision,
        folder,
        validity: validity.unwrap(),
        uid: uid.unwrap(),
        root: crate::mail_cache::root(conn)?,
        cached,
    })
}
pub(crate) fn request(
    conn: &mut rusqlite::Connection,
    account: &str,
    id: &str,
    revision: i64,
) -> Result<mail_store::MessageRow, AppError> {
    let tx = conn.transaction().map_err(|_| storage())?;
    let s = snapshot(&tx, account, id)?;
    if s.revision != revision {
        return Err(AppError::conflict("邮件已改变，请刷新后重试原文下载"));
    }
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE account_id=?1 AND id=?2",
        params![account, id],
    )
    .map_err(|_| storage())?;
    tx.execute("INSERT INTO imap_body_requests(account_id,message_id,generation,revision,state) VALUES (?1,?2,?3,?4,'queued') ON CONFLICT(account_id,message_id) DO UPDATE SET generation=excluded.generation,revision=excluded.revision,state='queued',error=NULL",params![account,id,s.generation,revision+1]).map_err(|_|storage())?;
    tx.commit().map_err(|_| storage())?;
    mail_store::message_by_id(conn, account, id)
}
fn fail(
    db: &SharedDb,
    account: &str,
    id: &str,
    error: &AppError,
    expected: Option<i64>,
) -> Result<(), AppError> {
    let mut conn = db.lock().map_err(|_| storage())?;
    let tx = conn.transaction().map_err(|_| storage())?;
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM imap_body_requests WHERE account_id=?1 AND message_id=?2",
            params![account, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| storage())?;
    if current.is_none() || expected.is_some_and(|expected| Some(expected) != current) {
        return Ok(());
    }
    tx.execute("UPDATE messages SET state_revision=state_revision+1 WHERE account_id=?1 AND id=?2 AND EXISTS(SELECT 1 FROM imap_body_requests r WHERE r.message_id=messages.id)",params![account,id]).map_err(|_|storage())?;
    tx.execute("UPDATE imap_body_requests SET state=?3,error=?4,revision=(SELECT state_revision FROM messages WHERE id=?2) WHERE account_id=?1 AND message_id=?2",params![account,id,if error.code=="network"{"queued"}else{"failed"},error.message]).map_err(|_|storage())?;
    tx.commit().map_err(|_| storage())
}
pub(crate) fn process<S: Read + Write>(
    db: &SharedDb,
    account: &str,
    session: &mut imap::Session<S>,
) -> Result<Vec<String>, AppError> {
    let ids = {
        let conn = db.lock().map_err(|_| storage())?;
        let mut stmt=conn.prepare("SELECT message_id FROM imap_body_requests WHERE account_id=?1 AND state='queued' ORDER BY message_id LIMIT 5").map_err(|_|storage())?;
        let ids = stmt
            .query_map([account], |r| r.get::<_, String>(0))
            .map_err(|_| storage())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage())?;
        ids
    };
    let mut warnings = vec![];
    for id in ids {
        let mut expected = None;
        let result = (|| -> Result<(), AppError> {
            let s = {
                let conn = db.lock().map_err(|_| storage())?;
                let request:(String,i64)=conn.query_row("SELECT generation,revision FROM imap_body_requests WHERE account_id=?1 AND message_id=?2",params![account,id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|storage())?;
                expected = Some(request.1);
                let s = snapshot(&conn, account, &id)?;
                if request != (s.generation.clone(), s.revision) {
                    return Err(AppError::conflict("下载期间账号或本机编辑改变，请重新请求"));
                }
                s
            };
            let boundary = session
                .examine(&s.folder)
                .map_err(|e| crate::imap_limits::network(e, "原文下载前无法核对文件夹"))?;
            if boundary.uid_validity != Some(s.validity) {
                return Err(AppError::conflict("原文 UIDVALIDITY 已改变，请先恢复定位"));
            }
            let mut fetched = crate::imap_download::fetch_range(
                session, s.uid, s.uid, &s.folder, s.validity, &s.root,
            )?;
            if fetched.len() != 1 {
                return Err(AppError::conflict("原 UID 不存在，未更改本机缓存"));
            }
            let mail = fetched.remove(0);
            if !mail_sync::matches_cached(&mail, &s.cached) {
                return Err(AppError::conflict(
                    "下载原文与历史缓存不符，未覆盖或升级证据",
                ));
            }
            crate::sync_control::check()?;
            let mut conn = db.lock().map_err(|_| storage())?;
            let tx = conn.transaction().map_err(|_| storage())?;
            let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN accounts a ON a.id=m.account_id JOIN imap_body_requests r ON r.account_id=m.account_id AND r.message_id=m.id WHERE m.account_id=?1 AND m.id=?2 AND m.state_revision=?3 AND r.revision=?3 AND a.sync_generation=?4 AND r.generation=?4)",params![s.account,s.id,s.revision,s.generation],|r|r.get(0)).map_err(|_|storage())?;
            if !current {
                return Err(AppError::conflict("下载完成前本机版本已改变，旧结果未应用"));
            }
            tx.execute("UPDATE messages SET raw_cache_key=?1,attachments_json=?2,content_hash=?3,header_message_id=?4,state_revision=state_revision+1 WHERE account_id=?5 AND id=?6",params![mail.raw_cache_key,serde_json::to_string(&mail.attachments).map_err(|_|storage())?,mail.content_hash,mail.header_message_id,account,id]).map_err(|_|storage())?;
            tx.execute(
                "DELETE FROM imap_body_requests WHERE account_id=?1 AND message_id=?2",
                params![account, id],
            )
            .map_err(|_| storage())?;
            tx.commit().map_err(|_| storage())
        })();
        if let Err(error) = result {
            fail(db, account, &id, &error, expected)?;
            if ["network", "cancelled"].contains(&error.code) {
                return Err(error);
            }
            warnings.push(error.message);
        }
    }
    Ok(warnings)
}
