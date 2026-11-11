//! Short IDLE slices reuse secure sessions while the same two FIFO workers stay fair.
use super::*;
use std::{
    collections::HashMap,
    sync::OnceLock,
    time::{Duration, Instant},
};
pub(super) type LiveSession =
    imap::Session<imap_writeback::TracedStream<native_tls::TlsStream<std::net::TcpStream>>>;
struct Pooled {
    condstore: bool,
    session: LiveSession,
    trace: imap_writeback::Trace,
    generation: String,
    idle: bool,
    dirty: bool,
    last_full: Instant,
    touched: Instant,
}
static POOL: OnceLock<Mutex<HashMap<String, Pooled>>> = OnceLock::new();
fn pool() -> &'static Mutex<HashMap<String, Pooled>> {
    POOL.get_or_init(Default::default)
}
pub(super) fn forget(id: &str) {
    let removed = pool().lock().ok().and_then(|mut p| p.remove(id));
    drop(removed);
}
fn take(id: &str) -> Result<Option<Pooled>, AppError> {
    pool()
        .lock()
        .map(|mut p| p.remove(id))
        .map_err(|_| AppError::storage("IMAP 会话缓存不可用"))
}
fn put(id: String, session: Pooled) -> Result<(), AppError> {
    let evicted = {
        let mut p = pool()
            .lock()
            .map_err(|_| AppError::storage("IMAP 会话缓存不可用"))?;
        let old = if p.len() >= 64 {
            let key = p
                .iter()
                .min_by_key(|(_, s)| s.touched)
                .map(|(id, _)| id.clone());
            key.and_then(|id| p.remove(&id))
        } else {
            None
        };
        p.insert(id, session);
        old
    };
    drop(evicted);
    Ok(())
}
fn local_work(db: &SharedDb, plan: &SyncPlan) -> Result<bool, AppError> {
    let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    ensure_current(&conn, plan)?;
    conn.query_row("SELECT EXISTS(SELECT 1 FROM message_changes WHERE account_id=?1 AND state IN ('queued','failed')) OR EXISTS(SELECT 1 FROM message_recovery WHERE account_id=?1 AND state='queued') OR EXISTS(SELECT 1 FROM imap_body_requests WHERE account_id=?1 AND state='queued')",[&plan.account_id],|r|r.get(0)).map_err(|_|AppError::storage("无法读取待同步操作"))
}
pub(super) fn idle_slice<S: Read + Write + imap::extensions::idle::SetReadTimeout>(
    session: &mut imap::Session<S>,
) -> Result<bool, imap::Error> {
    session
        .idle()?
        .wait_with_timeout(Duration::from_secs(5))
        .map(|outcome| outcome == imap::extensions::idle::WaitOutcome::MailboxChanged)
}
pub(super) fn run(db: &SharedDb, mut plan: SyncPlan) -> Result<SyncOutcome, AppError> {
    let cached = take(&plan.account_id)?;
    let mut live = match cached {
        Some(cached) if cached.generation == plan.generation => cached,
        cached => {
            drop(cached);
            let (mut session, trace) = open_session(db, &plan)?;
            let capabilities = session
                .capabilities()
                .map_err(|e| crate::imap_limits::network(e, "无法读取 IMAP 能力"))?;
            let idle = capabilities.has_str("IDLE");
            let condstore = capabilities.has_str("CONDSTORE");
            drop(capabilities);
            Pooled {
                condstore,
                session,
                trace,
                generation: plan.generation.clone(),
                idle,
                dirty: true,
                last_full: Instant::now(),
                touched: Instant::now(),
            }
        }
    };
    plan.condstore = live.condstore;
    imap_writeback::rebind_control(&live.trace)?;
    let full = live.dirty
        || plan.force_full
        || live.last_full.elapsed()
            >= Duration::from_secs(u64::from(plan.settings.auto_sync_seconds.max(60)));
    let mut outcome = if full {
        let outcome = sync_session(
            db,
            &plan,
            &mut live.session,
            Some(&live.trace),
            crate::imap_download::fetch_range,
        )?;
        live.dirty = outcome.more_mail;
        live.last_full = Instant::now();
        outcome
    } else if local_work(db, &plan)? {
        sync_session_mode(
            db,
            &plan,
            &mut live.session,
            Some(&live.trace),
            crate::imap_download::fetch_range,
            false,
        )?
    } else {
        SyncOutcome {
            synced_at: crate::now_utc(),
            ..Default::default()
        }
    };
    let enabled = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        ensure_current(&conn, &plan)?;
        crate::imap_settings::read(&conn, &plan.account_id)?.auto_sync_seconds > 0
    };
    if enabled && live.idle && !outcome.has_more && !outcome.retry_needed {
        live.session
            .examine("INBOX")
            .map_err(|e| crate::imap_limits::network(e, "无法准备收件箱通知"))?;
        let idle = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            idle_slice(&mut live.session)
        }));
        match idle {
            Ok(Ok(changed)) => {
                live.dirty = changed;
                outcome.has_more = true;
                outcome.listening = true;
            }
            Ok(Err(imap::Error::Bad(_) | imap::Error::No(_))) => {
                live.idle = false;
                outcome
                    .warnings
                    .push("服务器拒绝 IDLE，已使用定时同步。".into());
            }
            Ok(Err(error)) => return Err(crate::imap_limits::network(error, "IMAP 通知连接中断")),
            Err(_) => return Err(AppError::network("IMAP 通知响应无效，已保留同步批次")),
        }
    }
    crate::sync_control::check()?;
    if !outcome.retry_needed && (enabled || outcome.has_more) {
        live.touched = Instant::now();
        put(plan.account_id, live)?;
    } else {
        let _ = live.session.logout();
    }
    Ok(outcome)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap_writeback::tests::{session, step};
    #[test]
    fn idle_events_and_timeouts_finish_done_without_losing_the_tag_or_socket_bound() {
        for changed in [false, true] {
            let reply = if changed {
                "+ idling\r\n* 2 EXISTS\r\n"
            } else {
                "+ idling\r\n"
            };
            let (mut s, _, log) = session(
                vec![
                    step("IDLE", reply),
                    step("DONE", "{tag} OK done\r\n"),
                    step("NOOP", "{tag} OK noop\r\n"),
                ],
                None,
            );
            assert_eq!(idle_slice(&mut s).unwrap(), changed);
            s.noop().unwrap();
            assert_eq!(log.lock().unwrap().len(), 4);
        }
    }
    #[test]
    fn a_timeout_inside_an_idle_response_poison_stops_done_and_reuse() {
        let (mut s, _, log) = session(vec![step("IDLE", "+ idling\r\n* 2 EX")], None);
        assert!(idle_slice(&mut s).is_err());
        assert!(s.noop().is_err());
        assert_eq!(log.lock().unwrap().len(), 2);
    }
    #[test]
    fn reused_sessions_explicitly_rebind_to_the_new_jobs_control() {
        let first = crate::sync_control::Control::new();
        let _guard = crate::sync_control::enter(first.clone());
        let (mut s, t, _) = session(vec![step("NOOP", "{tag} OK noop\r\n")], None);
        let second = crate::sync_control::Control::new();
        let _next = crate::sync_control::enter(second.clone());
        imap_writeback::rebind_control(&t).unwrap();
        first.cancel();
        s.noop().unwrap();
        second.cancel();
        assert_eq!(
            crate::imap_limits::network(s.noop().unwrap_err(), "noop").code,
            "cancelled"
        );
    }
}
