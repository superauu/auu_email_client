//! Cooperative account-job cancellation. No global cancellation flag can cross accounts.
use crate::error::AppError;
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub(crate) struct Control {
    cancelled: Arc<AtomicBool>,
    deadline: Arc<OnceLock<Instant>>,
}
thread_local! {static CURRENT:RefCell<Option<Control>>=const {RefCell::new(None)};}
impl Control {
    pub(crate) fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Arc::new(OnceLock::new()),
        }
    }
    pub(crate) fn start(&self) {
        let _ = self.deadline.set(Instant::now() + Duration::from_secs(300));
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn check(&self) -> Result<(), AppError> {
        if self.cancelled() {
            Err(AppError {
                code: "cancelled",
                message: "同步已停止，已提交批次和本机修改保留".into(),
                retryable: false,
            })
        } else if self
            .deadline
            .get()
            .is_some_and(|deadline| Instant::now() >= *deadline)
        {
            Err(AppError::network("本轮同步达到时间预算，已提交批次保留"))
        } else {
            Ok(())
        }
    }
}
pub(crate) fn current() -> Option<Control> {
    CURRENT.with(|slot| slot.borrow().clone())
}
pub(crate) fn check() -> Result<(), AppError> {
    current().map_or(Ok(()), |c| c.check())
}
pub(crate) struct Guard(Option<Control>);
pub(crate) fn enter(control: Control) -> Guard {
    Guard(CURRENT.with(|slot| slot.replace(Some(control))))
}
impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with(|slot| slot.replace(self.0.take()));
    }
}
#[derive(Debug)]
pub(crate) struct Interrupted(pub bool);
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IMAP operation interrupted")
    }
}
impl std::error::Error for Interrupted {}
pub(crate) fn io_check(control: Option<&Control>) -> std::io::Result<()> {
    control.map_or(Ok(()), |c| c.check()).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            Interrupted(e.code == "cancelled"),
        )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn cancellation_is_scoped_and_returns_a_non_retrying_io_error() {
        let first = Control::new();
        let second = Control::new();
        let _guard = enter(first.clone());
        let mut stream =
            crate::imap_limits::LimitedStream::new(std::io::Cursor::new(b"* OK ready\r\n"));
        first.cancel();
        let mut bytes = vec![];
        let error = stream.read_to_end(&mut bytes).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionAborted);
        assert!(!second.cancelled());
        assert_eq!(
            crate::imap_limits::network(imap::Error::Io(error), "read").code,
            "cancelled"
        );
    }
    #[test]
    fn a_queued_job_has_no_deadline_until_it_starts_and_nested_guards_restore_the_owner() {
        let first = Control::new();
        assert!(first.deadline.get().is_none());
        first.start();
        assert!(first.deadline.get().is_some());
        let _outer = enter(first.clone());
        first.cancel();
        {
            let _inner = enter(Control::new());
            assert!(check().is_ok());
        }
        assert_eq!(check().unwrap_err().code, "cancelled");
        let expired = Control::new();
        expired
            .deadline
            .set(Instant::now() - Duration::from_secs(1))
            .unwrap();
        assert_eq!(expired.check().unwrap_err().code, "network");
    }
}
