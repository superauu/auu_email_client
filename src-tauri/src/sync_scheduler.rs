//! Process-local FIFO queue. Fixed workers bound network concurrency; no queue lock reaches a runner.
use crate::{error::AppError, mail_sync::SyncOutcome};
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex},
    thread,
};

const WORKERS: usize = 2;
const MAX_PENDING: usize = 128;
const MAX_FINISHED: usize = 64;
type Runner = dyn Fn(&str) -> Result<SyncOutcome, AppError> + Send + Sync + 'static;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
impl JobStatus {
    fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub cancel_requested: bool,
    pub id: String,
    pub account_id: String,
    pub status: JobStatus,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub outcome: Option<SyncOutcome>,
    pub error: Option<AppError>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub revision: u64,
    pub jobs: Vec<JobSnapshot>,
}

struct JobState {
    snapshot: JobSnapshot,
    result: Option<Result<SyncOutcome, AppError>>,
}
struct Job {
    control: crate::sync_control::Control,
    state: Mutex<JobState>,
    completed: Condvar,
}
pub struct Ticket(Arc<Job>);
impl Ticket {
    pub fn wait(self) -> Result<SyncOutcome, AppError> {
        let state = self
            .0
            .state
            .lock()
            .map_err(|_| AppError::storage("无法读取同步结果"))?;
        let state = self
            .0
            .completed
            .wait_while(state, |state| state.result.is_none())
            .map_err(|_| AppError::storage("同步结果等待未完成"))?;
        state
            .result
            .clone()
            .ok_or_else(|| AppError::storage("同步结果未就绪"))?
    }
}

struct Queue {
    revision: u64,
    pending: VecDeque<Arc<Job>>,
    jobs: VecDeque<Arc<Job>>,
    closed: bool,
}
struct Inner {
    queue: Mutex<Queue>,
    changed: Condvar,
    runner: Arc<Runner>,
}
pub struct SyncScheduler {
    inner: Arc<Inner>,
    owns_shutdown: bool,
}
impl Clone for SyncScheduler {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            owns_shutdown: false,
        }
    }
}

impl SyncScheduler {
    pub fn new(
        runner: impl Fn(&str) -> Result<SyncOutcome, AppError> + Send + Sync + 'static,
    ) -> Result<Self, AppError> {
        Self::with_workers(WORKERS, runner)
    }
    fn with_workers(
        workers: usize,
        runner: impl Fn(&str) -> Result<SyncOutcome, AppError> + Send + Sync + 'static,
    ) -> Result<Self, AppError> {
        let inner = Arc::new(Inner {
            queue: Mutex::new(Queue {
                revision: 0,
                pending: VecDeque::new(),
                jobs: VecDeque::new(),
                closed: false,
            }),
            changed: Condvar::new(),
            runner: Arc::new(runner),
        });
        let scheduler = Self {
            inner,
            owns_shutdown: true,
        };
        for index in 0..workers {
            let inner = scheduler.inner.clone();
            thread::Builder::new()
                .name(format!("mail-sync-{index}"))
                .spawn(move || worker(inner))
                .map_err(|_| AppError::storage("无法启动同步工作线程"))?;
        }
        Ok(scheduler)
    }

    pub fn enqueue(&self, ids: &[String]) -> Result<QueueSnapshot, AppError> {
        self.submit(ids).map(|(snapshot, _)| snapshot)
    }
    pub fn ticket(&self, id: String) -> Result<Ticket, AppError> {
        let (_, jobs) = self.submit(&[id])?;
        jobs.into_iter()
            .next()
            .ok_or_else(|| AppError::invalid("缺少同步账号"))
    }
    fn submit(&self, ids: &[String]) -> Result<(QueueSnapshot, Vec<Ticket>), AppError> {
        if ids.is_empty()
            || ids.len() > MAX_PENDING
            || ids.iter().any(|id| id.is_empty() || id.len() > 200)
        {
            return Err(AppError::invalid("每次请选择 1–128 个有效账号"));
        }
        let mut seen = HashSet::new();
        let ids: Vec<_> = ids.iter().filter(|id| seen.insert((*id).clone())).collect();
        let mut queue = self
            .inner
            .queue
            .lock()
            .map_err(|_| AppError::storage("同步队列不可用"))?;
        if queue.closed {
            return Err(AppError::storage("同步队列已关闭"));
        }
        let active = |id: &str, queue: &Queue| {
            queue
                .jobs
                .iter()
                .find(|job| {
                    let state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                    state.snapshot.account_id == id && state.snapshot.status.active()
                })
                .cloned()
        };
        let needed = ids.iter().filter(|id| active(id, &queue).is_none()).count();
        if queue.pending.len() + needed > MAX_PENDING {
            return Err(AppError::invalid("同步队列已满，请稍后重试"));
        }
        let mut tickets = Vec::new();
        for id in ids {
            let job = if let Some(job) = active(id, &queue) {
                job
            } else {
                let job = Arc::new(Job {
                    control: crate::sync_control::Control::new(),
                    state: Mutex::new(JobState {
                        snapshot: JobSnapshot {
                            cancel_requested: false,
                            id: uuid::Uuid::new_v4().to_string(),
                            account_id: id.clone(),
                            status: JobStatus::Queued,
                            queued_at: crate::now_utc(),
                            started_at: None,
                            finished_at: None,
                            outcome: None,
                            error: None,
                        },
                        result: None,
                    }),
                    completed: Condvar::new(),
                });
                queue.pending.push_back(job.clone());
                queue.jobs.push_back(job.clone());
                queue.revision += 1;
                job
            };
            tickets.push(Ticket(job));
        }
        let snapshot = snapshot(&queue);
        self.inner.changed.notify_all();
        Ok((snapshot, tickets))
    }
    pub fn snapshot(&self) -> Result<QueueSnapshot, AppError> {
        self.inner
            .queue
            .lock()
            .map(|queue| snapshot(&queue))
            .map_err(|_| AppError::storage("同步队列不可用"))
    }
    pub(crate) fn closed(&self) -> bool {
        self.inner.queue.lock().map_or(true, |q| q.closed)
    }
    pub fn cancel(&self, id: &str) -> Result<QueueSnapshot, AppError> {
        let mut queue = self
            .inner
            .queue
            .lock()
            .map_err(|_| AppError::storage("同步队列不可用"))?;
        let index = queue.pending.iter().position(|job| {
            job.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .snapshot
                .id
                == id
        });
        if let Some(index) = index {
            let job = queue
                .pending
                .remove(index)
                .ok_or_else(|| AppError::conflict("任务已开始"))?;
            finish(
                &job,
                JobStatus::Cancelled,
                Err(AppError {
                    code: "cancelled",
                    message: "已取消排队同步".into(),
                    retryable: false,
                }),
            );
            queue.revision += 1;
            prune(&mut queue);
        } else {
            let state = queue.jobs.iter().find_map(|job| {
                let state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                (state.snapshot.id == id).then_some(state.snapshot.status)
            });
            if state == Some(JobStatus::Running) {
                if let Some(job) = queue.jobs.iter().find(|job| {
                    job.state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .snapshot
                        .id
                        == id
                }) {
                    job.control.cancel();
                    job.state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .snapshot
                        .cancel_requested = true;
                }
                queue.revision += 1;
            }
            if state.is_none() {
                return Err(AppError::invalid("找不到该同步任务"));
            }
        }
        Ok(snapshot(&queue))
    }
    pub(crate) fn cancel_account(&self, account: &str) -> Result<(), AppError> {
        let ids = self
            .snapshot()?
            .jobs
            .into_iter()
            .filter(|job| {
                job.account_id == account
                    && matches!(job.status, JobStatus::Queued | JobStatus::Running)
            })
            .map(|job| job.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.cancel(&id)?;
        }
        Ok(())
    }
}

impl Drop for SyncScheduler {
    fn drop(&mut self) {
        if !self.owns_shutdown {
            return;
        }
        let mut queue = self
            .inner
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.closed = true;
        for job in queue.pending.drain(..) {
            finish(
                &job,
                JobStatus::Cancelled,
                Err(AppError::storage("应用退出，排队任务已停止")),
            );
        }
        self.inner.changed.notify_all();
        // Network workers are detached; app shutdown never waits for a stalled socket.
    }
}
fn snapshot(queue: &Queue) -> QueueSnapshot {
    QueueSnapshot {
        revision: queue.revision,
        jobs: queue
            .jobs
            .iter()
            .map(|job| {
                job.state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .snapshot
                    .clone()
            })
            .collect(),
    }
}
fn finish(job: &Job, status: JobStatus, result: Result<SyncOutcome, AppError>) {
    let mut state = job.state.lock().unwrap_or_else(|error| error.into_inner());
    state.snapshot.status = status;
    state.snapshot.finished_at = Some(crate::now_utc());
    state.snapshot.outcome = result.as_ref().ok().cloned();
    state.snapshot.error = result.as_ref().err().cloned();
    state.result = Some(result);
    job.completed.notify_all();
}
fn prune(queue: &mut Queue) {
    while queue
        .jobs
        .iter()
        .filter(|job| {
            !job.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .snapshot
                .status
                .active()
        })
        .count()
        > MAX_FINISHED
    {
        if let Some(index) = queue.jobs.iter().position(|job| {
            !job.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .snapshot
                .status
                .active()
        }) {
            queue.jobs.remove(index);
        } else {
            break;
        }
    }
}
fn worker(inner: Arc<Inner>) {
    loop {
        let job = {
            let queue = inner
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut queue = inner
                .changed
                .wait_while(queue, |queue| queue.pending.is_empty() && !queue.closed)
                .unwrap_or_else(|error| error.into_inner());
            if queue.closed {
                return;
            }
            let Some(job) = queue.pending.pop_front() else {
                continue;
            };
            {
                let mut state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                state.snapshot.status = JobStatus::Running;
                state.snapshot.started_at = Some(crate::now_utc());
            }
            queue.revision += 1;
            job
        };
        let id = job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot
            .account_id
            .clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            job.control.start();
            let _guard = crate::sync_control::enter(job.control.clone());
            job.control.check()?;
            (inner.runner)(&id)
        }))
        .unwrap_or_else(|_| {
            Err(AppError {
                code: "internal",
                message: "同步任务异常结束，请重试。".into(),
                retryable: true,
            })
        });
        let result = if job.control.cancelled() {
            Err(AppError {
                code: "cancelled",
                message: "同步已停止，已提交批次保留".into(),
                retryable: false,
            })
        } else {
            result
        };
        let mut queue = inner
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        finish(
            &job,
            if result.as_ref().err().is_some_and(|e| e.code == "cancelled") {
                JobStatus::Cancelled
            } else if result.is_ok() {
                JobStatus::Succeeded
            } else {
                JobStatus::Failed
            },
            result,
        );
        queue.revision += 1;
        prune(&mut queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };
    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn concurrency_is_bounded_and_duplicate_accounts_share_one_job() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release = Mutex::new(release_rx);
        let a = active.clone();
        let p = peak.clone();
        let scheduler = SyncScheduler::new(move |id| {
            let count = a.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(count, Ordering::SeqCst);
            started_tx.send(id.to_string()).unwrap();
            release
                .lock()
                .unwrap()
                .recv_timeout(WAIT)
                .map_err(|_| AppError::network("fixture timeout"))?;
            a.fetch_sub(1, Ordering::SeqCst);
            Ok(SyncOutcome::default())
        })
        .unwrap();
        let first = scheduler.ticket("a".into()).unwrap();
        let second = scheduler.ticket("b".into()).unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        let before = scheduler.snapshot().unwrap();
        let duplicate = scheduler.enqueue(&["a".into(), "a".into()]).unwrap();
        assert_eq!(before.jobs.len(), duplicate.jobs.len());
        let third = scheduler.ticket("c".into()).unwrap();
        assert_eq!(
            scheduler
                .snapshot()
                .unwrap()
                .jobs
                .iter()
                .filter(|job| job.status == JobStatus::Queued)
                .count(),
            1
        );
        for _ in 0..3 {
            release_tx.send(()).unwrap();
        }
        first.wait().unwrap();
        second.wait().unwrap();
        third.wait().unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fifo_cancellation_and_failure_do_not_block_the_next_account() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release = Mutex::new(release_rx);
        let scheduler = SyncScheduler::with_workers(1, move |id| {
            started_tx.send(id.to_string()).unwrap();
            if id == "a" {
                release.lock().unwrap().recv_timeout(WAIT).unwrap();
                return Err(AppError::network("offline"));
            }
            Ok(SyncOutcome::default())
        })
        .unwrap();
        let a = scheduler.ticket("a".into()).unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        let b = scheduler.ticket("b".into()).unwrap();
        let c = scheduler.ticket("c".into()).unwrap();
        let snapshot = scheduler.snapshot().unwrap();
        let c_id = snapshot
            .jobs
            .iter()
            .find(|job| job.account_id == "c")
            .unwrap()
            .id
            .clone();
        let a_id = snapshot
            .jobs
            .iter()
            .find(|job| job.account_id == "a")
            .unwrap()
            .id
            .clone();
        assert!(
            scheduler
                .cancel(&a_id)
                .unwrap()
                .jobs
                .iter()
                .find(|job| job.id == a_id)
                .unwrap()
                .cancel_requested
        );
        scheduler.cancel(&c_id).unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(a.wait().unwrap_err().code, "cancelled");
        b.wait().unwrap();
        assert_eq!(c.wait().unwrap_err().code, "cancelled");
        assert_eq!(started_rx.recv_timeout(WAIT).unwrap(), "b");
        assert!(started_rx.try_recv().is_err());
        let revision = scheduler.snapshot().unwrap().revision;
        scheduler.ticket("b".into()).unwrap().wait().unwrap();
        assert!(scheduler.snapshot().unwrap().revision > revision);
    }

    #[test]
    fn a_panicking_runner_does_not_kill_a_worker() {
        let scheduler = SyncScheduler::with_workers(1, |id| {
            if id == "panic" {
                panic!("fixture panic");
            }
            Ok(SyncOutcome::default())
        })
        .unwrap();
        assert_eq!(
            scheduler
                .ticket("panic".into())
                .unwrap()
                .wait()
                .unwrap_err()
                .code,
            "internal"
        );
        scheduler.ticket("healthy".into()).unwrap().wait().unwrap();
    }

    #[test]
    fn completed_history_is_bounded_and_invalid_batches_are_atomic() {
        let scheduler = SyncScheduler::with_workers(1, |_| Ok(SyncOutcome::default())).unwrap();
        assert!(scheduler.enqueue(&[]).is_err());
        assert!(scheduler.enqueue(&vec!["a".into(); 129]).is_err());
        assert!(scheduler.snapshot().unwrap().jobs.is_empty());
        for id in 0..80 {
            scheduler.ticket(format!("a-{id}")).unwrap().wait().unwrap();
        }
        assert!(scheduler.snapshot().unwrap().jobs.len() <= MAX_FINISHED);
    }

    #[test]
    fn full_queue_rejects_the_entire_new_batch() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release = Mutex::new(release_rx);
        let scheduler = SyncScheduler::with_workers(1, move |id| {
            if id == "busy" {
                started_tx.send(()).unwrap();
                release.lock().unwrap().recv_timeout(WAIT).unwrap();
            }
            Ok(SyncOutcome::default())
        })
        .unwrap();
        let busy = scheduler.ticket("busy".into()).unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        let full: Vec<String> = (0..MAX_PENDING).map(|i| format!("queued-{i}")).collect();
        scheduler.enqueue(&full).unwrap();
        let rejected = scheduler
            .enqueue(&["overflow-1".into(), "overflow-2".into()])
            .is_err();
        let count = scheduler.snapshot().unwrap().jobs.len();
        release_tx.send(()).unwrap();
        busy.wait().unwrap();
        assert!(rejected);
        assert_eq!(count, MAX_PENDING + 1);
    }
}
