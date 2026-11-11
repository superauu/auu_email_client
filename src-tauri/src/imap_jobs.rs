//! Durable account requests. The existing scheduler remains the only network executor.
use crate::{
    error::AppError,
    mail_sync::{SharedDb, SyncOutcome},
    sync_scheduler::SyncScheduler,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, OptionalExtension};
#[cfg(test)]
#[path = "imap_jobs_tests.rs"]
mod tests;
fn storage() -> AppError {
    AppError::storage("IMAP 持久请求不可用")
}
fn after(now: &str, seconds: i64) -> Result<String, AppError> {
    Ok(
        (DateTime::parse_from_rfc3339(now).map_err(|_| storage())? + Duration::seconds(seconds))
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}
pub(crate) fn recover(conn: &rusqlite::Connection) -> Result<(), AppError> {
    conn.execute("UPDATE imap_sync_requests SET state='queued',due_at=?1,error='上次同步中断，继续上个已提交批次。',updated_at=?1 WHERE state='running'",[crate::now_utc()]).map_err(|_|storage())?;
    Ok(())
}
pub(crate) fn request(conn: &mut rusqlite::Connection, ids: &[String]) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(|_| storage())?;
    let now = crate::now_utc();
    for id in ids {
        tx.execute("INSERT INTO imap_sync_requests(account_id,generation,state,due_at,updated_at) SELECT id,sync_generation,'queued',?2,?2 FROM accounts WHERE id=?1 ON CONFLICT(account_id) DO UPDATE SET generation=excluded.generation,state=CASE WHEN imap_sync_requests.generation=excluded.generation AND imap_sync_requests.state='running' THEN 'running' ELSE 'queued' END,origin='manual',request_version=imap_sync_requests.request_version+1,attempts=0,due_at=excluded.due_at,error=NULL,updated_at=excluded.updated_at",params![id,now]).map_err(|_|storage())?;
    }
    tx.commit().map_err(|_| storage())
}
pub(crate) fn pause(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    conn.execute("UPDATE imap_sync_requests SET state='paused',request_version=request_version+1,error='用户已停止同步；手动同步可重新开始。',updated_at=?2 WHERE account_id=?1",params![id,crate::now_utc()]).map_err(|_|storage())?;
    Ok(())
}
#[cfg(test)]
fn complete(
    conn: &rusqlite::Connection,
    id: &str,
    generation: &str,
    result: &Result<SyncOutcome, AppError>,
    now: &str,
) -> Result<(), AppError> {
    complete_version(conn, id, generation, None, result, now)
}
fn complete_version(
    conn: &rusqlite::Connection,
    id: &str,
    generation: &str,
    version: Option<i64>,
    result: &Result<SyncOutcome, AppError>,
    now: &str,
) -> Result<(), AppError> {
    let active:Option<(u32,String,String,i64)>=conn.query_row("SELECT r.attempts,r.state,a.sync_generation,r.request_version FROM imap_sync_requests r JOIN accounts a ON a.id=r.account_id WHERE r.account_id=?1 AND r.generation=?2",params![id,generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|storage())?;
    let Some((attempts, state, current, current_version)) = active else {
        return Ok(());
    };
    if state == "paused" || current != generation {
        return pause(conn, id);
    }
    if version.is_some_and(|version| version != current_version) {
        if state == "running" {
            conn.execute("UPDATE imap_sync_requests SET state='queued',origin='manual',due_at=?2,attempts=0 WHERE account_id=?1 AND request_version=?3",params![id,now,current_version]).map_err(|_|storage())?;
        }
        return Ok(());
    }
    let settings = crate::imap_settings::read(conn, id)?;
    let pending_read_only:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM message_recovery WHERE account_id=?1 AND state='queued') OR EXISTS(SELECT 1 FROM imap_body_requests WHERE account_id=?1 AND state='queued')",[id],|r|r.get(0)).map_err(|_|storage())?;
    let (state, origin, due, error, reset) = match result {
        Ok(outcome) if outcome.retry_needed => (
            if attempts < 8 { "failed" } else { "paused" },
            "continuation",
            after(
                now,
                (5i64 * 2i64.pow(attempts.saturating_sub(1).min(7))).min(900),
            )?,
            Some("部分文件夹同步失败，已提交数据保留。".into()),
            false,
        ),
        Ok(outcome) if outcome.has_more || pending_read_only => {
            ("queued", "continuation", after(now, 1)?, None, true)
        }
        Ok(_) if settings.auto_sync_seconds > 0 => (
            "queued",
            "auto",
            after(now, i64::from(settings.auto_sync_seconds))?,
            None,
            true,
        ),
        Ok(_) => {
            conn.execute(
                "DELETE FROM imap_sync_requests WHERE account_id=?1 AND generation=?2",
                params![id, generation],
            )
            .map_err(|_| storage())?;
            return Ok(());
        }
        Err(error) if error.retryable && error.code == "network" && attempts < 8 => (
            "failed",
            "continuation",
            after(
                now,
                (5i64 * 2i64.pow(attempts.saturating_sub(1).min(7))).min(900),
            )?,
            Some(error.message.clone()),
            false,
        ),
        Err(error) => (
            "paused",
            "manual",
            now.into(),
            Some(error.message.clone()),
            false,
        ),
    };
    conn.execute("UPDATE imap_sync_requests SET state=?3,origin=?4,due_at=?5,error=?6,attempts=CASE WHEN ?7 THEN 0 ELSE attempts END,updated_at=?8 WHERE account_id=?1 AND generation=?2",params![id,generation,state,origin,due,error,reset,now]).map_err(|_|storage())?;
    Ok(())
}
pub(crate) fn run(db: &SharedDb, id: &str) -> Result<SyncOutcome, AppError> {
    let (generation, version) = {
        let conn = db.lock().map_err(|_| storage())?;
        let row:Option<(String,String,String,i64)>=conn.query_row("SELECT r.generation,a.sync_generation,r.state,r.request_version FROM imap_sync_requests r JOIN accounts a ON a.id=r.account_id WHERE r.account_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|storage())?;
        let Some((generation, current, state, version)) = row else {
            return Err(AppError::invalid("同步请求已移除"));
        };
        if generation != current || state == "paused" {
            pause(&conn, id)?;
            return Err(AppError::conflict(
                "账号连接已改变或同步已停止，请手动重新同步。",
            ));
        }
        conn.execute("UPDATE imap_sync_requests SET state='running',attempts=attempts+1,updated_at=?2 WHERE account_id=?1",params![id,crate::now_utc()]).map_err(|_|storage())?;
        (generation, version)
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::mail_sync::sync_account_generation(db, id, &generation)
    }))
    .unwrap_or_else(|_| {
        Err(AppError::invalid(
            "同步任务异常中断，已提交批次保留，请手动重试",
        ))
    })
    .and_then(|outcome| {
        crate::sync_control::check()?;
        Ok(outcome)
    });
    let result = if crate::sync_control::current().is_some_and(|c| c.cancelled()) {
        Err(AppError {
            code: "cancelled",
            message: "同步已停止，已提交批次保留".into(),
            retryable: false,
        })
    } else {
        result
    };
    let conn = db.lock().map_err(|_| storage())?;
    complete_version(
        &conn,
        id,
        &generation,
        Some(version),
        &result,
        &crate::now_utc(),
    )?;
    result
}
pub(crate) fn spawn(db: SharedDb, scheduler: SyncScheduler) -> Result<(), AppError> {
    std::thread::Builder::new().name("imap-persistent-dispatch".into()).spawn(move||{
        while !scheduler.closed(){
            let active=scheduler.snapshot().map(|snapshot|snapshot.jobs.into_iter().filter(|job|matches!(job.status,crate::sync_scheduler::JobStatus::Running|crate::sync_scheduler::JobStatus::Queued)).map(|job|job.account_id).collect::<std::collections::HashSet<_>>()).unwrap_or_default();
            let due=(||->Result<Vec<String>,AppError>{
                let conn=db.lock().map_err(|_|storage())?;let now=Utc::now().to_rfc3339_opts(SecondsFormat::Secs,true);
                conn.execute("INSERT OR IGNORE INTO imap_sync_requests(account_id,generation,state,origin,due_at,updated_at) SELECT a.id,a.sync_generation,'queued','auto',?1,?1 FROM accounts a JOIN imap_settings s ON s.account_id=a.id WHERE s.auto_sync_seconds>0 AND a.provider='imap' AND COALESCE(json_extract(CASE WHEN json_valid(a.profile_json) THEN a.profile_json ELSE '{}' END,'$.status'),'')!='demo'",[&now]).map_err(|_|storage())?;
                conn.execute("DELETE FROM imap_sync_requests WHERE origin='auto' AND state='queued' AND EXISTS(SELECT 1 FROM imap_settings s WHERE s.account_id=imap_sync_requests.account_id AND s.auto_sync_seconds=0)",[]).map_err(|_|storage())?;
                let mut stmt=conn.prepare("SELECT account_id FROM imap_sync_requests WHERE state IN ('queued','failed') AND due_at<=?1 ORDER BY due_at,account_id LIMIT 128").map_err(|_|storage())?;
                let result=stmt.query_map([now],|r|r.get::<_,String>(0)).map_err(|_|storage())?.collect::<Result<Vec<_>,_>>().map_err(|_|storage())?;Ok(result)
            })();
            if let Ok(due)=due{for id in due{if !active.contains(&id){let _=scheduler.enqueue(&[id]);}}}
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }).map_err(|_|storage())?;
    Ok(())
}
