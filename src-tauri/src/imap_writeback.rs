//! Durable intent replay. All calls to the IMAP session are outside the database guard.
use crate::{
    error::AppError,
    mail_sync::SharedDb,
    message_changes::{self, Intent},
};
use rusqlite::params;
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};

#[derive(Default, Debug)]
pub struct MoveTrace {
    wire: Option<crate::imap_limits::ControlSwitch>,
    armed: bool,
    tag: Option<String>,
    source: u32,
    mapping: Option<(u32, u32)>,
    invalid: bool,
    line: Vec<u8>,
    literal: usize,
    oversized: bool,
    outgoing: Vec<u8>,
}
pub type Trace = Arc<Mutex<MoveTrace>>;
/// imap 2.x discards tagged response codes. Capture COPYUID without interpreting body literals.
pub struct TracedStream<S> {
    inner: crate::imap_limits::LimitedStream<S>,
    trace: Trace,
}
impl<S> TracedStream<S> {
    pub fn new(inner: S) -> (Self, Trace) {
        let inner = crate::imap_limits::LimitedStream::new(inner);
        let trace = Arc::new(Mutex::new(MoveTrace {
            wire: Some(inner.control_handle()),
            ..Default::default()
        }));
        (
            Self {
                inner,
                trace: trace.clone(),
            },
            trace,
        )
    }
}
pub(crate) fn rebind_control(trace: &Trace) -> Result<(), AppError> {
    let wire = trace
        .lock()
        .map_err(|_| AppError::storage("IMAP 连接状态不可用"))?
        .wire
        .clone()
        .ok_or_else(|| AppError::storage("IMAP 连接控制器缺失"))?;
    *wire
        .lock()
        .map_err(|_| AppError::storage("IMAP 连接控制器不可用"))? = crate::sync_control::current();
    Ok(())
}
impl<S: imap::extensions::idle::SetReadTimeout> imap::extensions::idle::SetReadTimeout
    for TracedStream<S>
{
    fn set_read_timeout(
        &mut self,
        timeout: Option<std::time::Duration>,
    ) -> imap::error::Result<()> {
        self.inner.set_read_timeout(timeout)
    }
}
fn single_uid(text: &str) -> Option<u32> {
    let value = if let Some((a, b)) = text.split_once(':') {
        if a != b {
            return None;
        }
        a
    } else {
        text
    };
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u32>().ok().filter(|uid| *uid > 0)
}
impl MoveTrace {
    fn arm(&mut self, source: u32) {
        self.armed = true;
        self.tag = None;
        self.source = source;
        self.mapping = None;
        self.invalid = false;
        self.outgoing.clear();
    }
    fn sent(&mut self, bytes: &[u8]) {
        if !self.armed {
            return;
        }
        for byte in bytes {
            if self.outgoing.len() < 65536 {
                self.outgoing.push(*byte);
            }
            if *byte == b'\n' {
                if let Ok(line) = std::str::from_utf8(&self.outgoing) {
                    let mut words = line.split_ascii_whitespace();
                    if let (Some(tag), Some("UID"), Some("MOVE")) =
                        (words.next(), words.next(), words.next())
                    {
                        self.tag = Some(tag.to_owned());
                    }
                }
                self.outgoing.clear();
            }
        }
    }
    fn received(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self.literal > 0 {
                let n = self.literal.min(bytes.len());
                self.literal -= n;
                bytes = &bytes[n..];
                continue;
            }
            let byte = &bytes[0];
            bytes = &bytes[1..];
            if self.line.len() == 65536 {
                self.line.clear();
                self.oversized = true;
            }
            self.line.push(*byte);
            if *byte != b'\n' {
                continue;
            }
            let line = String::from_utf8_lossy(&self.line);
            // IMAP literal marker at the end of a protocol line; never scan its bytes.
            if let Some(prefix) = line.trim_end().strip_suffix('}') {
                if let Some((_, n)) = prefix.rsplit_once('{') {
                    if let Ok(n) = n.trim_end_matches('+').parse::<usize>() {
                        self.literal = n;
                    }
                }
            }
            if self.armed && !self.oversized {
                let mut words = line.split_ascii_whitespace();
                let first = words.next();
                let status = words.next();
                if status == Some("OK")
                    && (first == Some("*") || first == self.tag.as_deref())
                    && words.next() == Some("[COPYUID")
                {
                    let parsed = (|| {
                        let validity = single_uid(words.next()?)?;
                        let source = single_uid(words.next()?)?;
                        let target = single_uid(words.next()?.strip_suffix(']')?)?;
                        (source == self.source).then_some((validity, target))
                    })();
                    match parsed {
                        Some(map) if self.mapping.is_none_or(|prior| prior == map) => {
                            self.mapping = Some(map)
                        }
                        _ => self.invalid = true,
                    }
                }
            }
            self.line.clear();
            self.oversized = false;
        }
    }
}
impl<S: Read> Read for TracedStream<S> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buffer)?;
        self.trace
            .lock()
            .map_err(|_| std::io::Error::other("IMAP trace unavailable"))?
            .received(&buffer[..n]);
        Ok(n)
    }
}
impl<S: Write> Write for TracedStream<S> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(bytes)?;
        self.trace
            .lock()
            .map_err(|_| std::io::Error::other("IMAP trace unavailable"))?
            .sent(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn state(
    db: &SharedDb,
    intent: &Intent,
    status: &str,
    error: Option<&str>,
    target: Option<&str>,
) -> Result<bool, AppError> {
    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    message_changes::transition(&mut conn, intent, status, error, target)
}
/// At most 200 intents per manually requested sync. Changes made during replay remain durable.
pub fn flush<S: Read + Write>(
    db: &SharedDb,
    account: &str,
    session: &mut imap::Session<S>,
    archive: Option<&str>,
    trace: Option<&Trace>,
) -> Result<Vec<String>, AppError> {
    let intents = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        message_changes::pending(&conn, account)?
    };
    if intents.is_empty() {
        return Ok(vec![]);
    }
    let can_move = session
        .capabilities()
        .map_err(|error| crate::imap_limits::network(error, "无法读取 IMAP 回写能力"))?
        .has_str("MOVE");
    let mut warnings = vec![];
    for intent in intents {
        {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            if !message_changes::current(&conn, &intent)? {
                let paused:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM message_recovery WHERE message_id=?1 AND state NOT IN ('restored','local'))",[&intent.message_id],|r|r.get(0)).map_err(|_|AppError::storage("无法检查恢复请求"))?;
                if paused {
                    continue;
                }
                drop(conn);
                state(
                    db,
                    &intent,
                    "conflict",
                    Some("账号或本地修改已改变，请核对后保留本机。"),
                    None,
                )?;
                continue;
            }
        }
        if intent.field == "archive" && (!can_move || archive.is_none() || trace.is_none()) {
            state(
                db,
                &intent,
                "conflict",
                Some("服务器未提供 MOVE 或唯一归档文件夹；已保留本机归档。"),
                None,
            )?;
            warnings.push("归档未回写：服务器能力或目标文件夹无法确认。".into());
            continue;
        }
        let selected = match session.select(&intent.folder) {
            Ok(folder) => folder,
            Err(_) => {
                state(
                    db,
                    &intent,
                    "failed",
                    Some("无法打开原文件夹，下次同步重试。"),
                    None,
                )?;
                return Err(AppError::network("邮件状态回写时连接失败"));
            }
        };
        if selected.uid_validity != Some(intent.validity) {
            state(
                db,
                &intent,
                "conflict",
                Some("UIDVALIDITY 已改变，禁止向新邮件套用旧修改。"),
                None,
            )?;
            warnings.push("邮件定位已过期，本机修改需要核对。".into());
            continue;
        }
        let found = session
            .uid_fetch(intent.uid.to_string(), "(UID FLAGS)")
            .map_err(|error| crate::imap_limits::network(error, "无法核对待回写邮件"))?;
        if found.len() != 1 || found[0].uid != Some(intent.uid) {
            state(
                db,
                &intent,
                "conflict",
                Some("原 UID 已不存在，可能已被其他客户端移动或删除。"),
                None,
            )?;
            warnings.push("原邮件已移动或删除，保留本机修改。".into());
            continue;
        }
        drop(found);
        // Recheck immediately before dispatch; a concurrent newer intent must never be acknowledged here.
        {
            let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            if !message_changes::current(&conn, &intent)? {
                continue;
            }
            if !message_changes::transition(&mut conn, &intent, "applying", None, archive)? {
                continue;
            }
        }
        if intent.field != "archive" {
            let (flag, add) = if intent.field == "unread" {
                ("\\Seen", !intent.value)
            } else {
                ("\\Flagged", intent.value)
            };
            match session.uid_store(
                intent.uid.to_string(),
                format!("{}FLAGS.SILENT ({flag})", if add { "+" } else { "-" }),
            ) {
                Ok(_) => {
                    let observed = match session.uid_fetch(intent.uid.to_string(), "(UID FLAGS)") {
                        Ok(values) => values,
                        Err(_) => {
                            state(
                                db,
                                &intent,
                                "failed",
                                Some("标记已发出但确认失败，下次同步可安全重试。"),
                                None,
                            )?;
                            return Err(AppError::network("邮件标记确认失败；本机修改已保存"));
                        }
                    };
                    let verified = observed.len() == 1
                        && observed[0].uid == Some(intent.uid)
                        && if intent.field == "unread" {
                            !observed[0].flags().contains(&imap::types::Flag::Seen) == intent.value
                        } else {
                            observed[0].flags().contains(&imap::types::Flag::Flagged)
                                == intent.value
                        };
                    if !verified {
                        state(
                            db,
                            &intent,
                            "conflict",
                            Some("服务器标记与请求不一致或原 UID 已消失，请核对。"),
                            None,
                        )?;
                        warnings.push("服务器未确认邮件标记，保留本机修改。".into());
                        continue;
                    }
                    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
                    let result = message_changes::acknowledge_flag(&mut conn, &intent);
                    drop(conn);
                    if let Err(error) = result {
                        state(
                            db,
                            &intent,
                            "failed",
                            Some("服务器标记已处理，本机确认失败；下次同步核对重试。"),
                            None,
                        )?;
                        return Err(error);
                    }
                }
                Err(_) => {
                    state(
                        db,
                        &intent,
                        "failed",
                        Some("标记回写未确认，下次同步可安全重试。"),
                        None,
                    )?;
                    return Err(AppError::network("邮件标记回写未确认；本机修改已保存"));
                }
            }
        } else {
            let target = archive.expect("checked archive");
            let trace = trace.expect("checked trace");
            if target == intent.folder {
                state(
                    db,
                    &intent,
                    "conflict",
                    Some("邮件已在目标文件夹，请核对本机归档。"),
                    None,
                )?;
                continue;
            }
            trace
                .lock()
                .map_err(|_| AppError::storage("归档响应不可用"))?
                .arm(intent.uid);
            let result = session.uid_mv(intent.uid.to_string(), target);
            let mapping = {
                let mut capture = trace
                    .lock()
                    .map_err(|_| AppError::storage("归档响应不可用"))?;
                capture.armed = false;
                if capture.invalid {
                    None
                } else {
                    capture.mapping
                }
            };
            match (result, mapping) {
                (Ok(()), Some((validity, uid))) => {
                    if let Err(error) = acknowledge_move(db, &intent, target, validity, uid) {
                        state(
                            db,
                            &intent,
                            "unknown",
                            Some("服务器已执行移动，但本机确认失败；请核对，禁止盲目重试。"),
                            Some(target),
                        )?;
                        return Err(error);
                    }
                }
                (result, _) => {
                    state(
                        db,
                        &intent,
                        "unknown",
                        Some("归档命令已发出，结果或目标 UID 未确认；不会自动重试，请核对服务器。"),
                        Some(target),
                    )?;
                    warnings.push("归档结果待核对，已停止自动重试。".into());
                    // A failed command can mean a broken stream; do not issue more commands on it.
                    if result.is_err() {
                        return Err(AppError::network("归档结果未确认；本机归档仍保留"));
                    }
                }
            }
        }
    }
    Ok(warnings)
}

fn acknowledge_move(
    db: &SharedDb,
    intent: &Intent,
    target: &str,
    validity: u32,
    uid: u32,
) -> Result<(), AppError> {
    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法保存归档结果"))?;
    if !message_changes::current(&tx, intent)? {
        return Err(AppError::conflict("账号或归档修改已改变，归档结果需核对。"));
    }
    let folder = format!("{}:folder:{target}", intent.account_id);
    // Preserve the stable local message id and event references; only the verified remote locator changes.
    tx.execute("UPDATE messages SET remote_id=?1,remote_folder=?2,remote_uid_validity=?3,remote_uid=?4,remote_generation=?5,state_revision=state_revision+1 WHERE id=?6 AND account_id=?7",
      params![format!("imap:archive:{validity}:{uid}"),target,validity,uid,intent.generation,intent.message_id,intent.account_id]).map_err(|_|AppError::storage("归档定位未能保存，请核对服务器"))?;
    tx.execute(
        "DELETE FROM message_folders WHERE account_id=?1 AND message_id=?2",
        params![intent.account_id, intent.message_id],
    )
    .map_err(|_| AppError::storage("无法更新归档文件夹"))?;
    tx.execute(
        "INSERT INTO message_folders VALUES (?1,?2,?3)",
        params![intent.account_id, intent.message_id, folder],
    )
    .map_err(|_| AppError::storage("无法保存归档文件夹"))?;
    tx.execute("DELETE FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field='archive' AND revision=?3",params![intent.account_id,intent.message_id,intent.revision]).map_err(|_|AppError::storage("无法确认归档"))?;
    tx.execute("UPDATE message_changes SET source_folder=?1,source_validity=?2,source_uid=?3 WHERE account_id=?4 AND message_id=?5 AND state IN ('queued','failed')",params![target,validity,uid,intent.account_id,intent.message_id]).map_err(|_|AppError::storage("无法更新待回写定位"))?;
    tx.commit()
        .map_err(|_| AppError::storage("归档结果提交失败"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;
    type Step = (String, String);
    pub(crate) type Hook = Box<dyn FnMut(&str) + Send>;
    pub(crate) struct Script {
        idle_tag: Option<String>,
        pending: VecDeque<u8>,
        outgoing: Vec<u8>,
        steps: VecDeque<Step>,
        log: Arc<Mutex<Vec<String>>>,
        hook: Option<Hook>,
    }
    impl std::fmt::Debug for Script {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Scripted IMAP")
        }
    }
    impl Read for Script {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.pending.is_empty() && self.idle_tag.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "scripted idle timeout",
                ));
            }
            let count = buffer.len().min(self.pending.len());
            for item in &mut buffer[..count] {
                *item = self.pending.pop_front().unwrap();
            }
            Ok(count)
        }
    }
    impl Write for Script {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            for byte in bytes {
                self.outgoing.push(*byte);
                if *byte == b'\n' {
                    let command = String::from_utf8(std::mem::take(&mut self.outgoing)).unwrap();
                    let tag = if command.trim() == "DONE" {
                        self.idle_tag.take().expect("DONE without IDLE")
                    } else {
                        let tag = command.split_whitespace().next().unwrap().to_owned();
                        if command.contains(" IDLE") {
                            self.idle_tag = Some(tag.clone());
                        }
                        tag
                    };
                    let (expected, response) =
                        self.steps.pop_front().expect("unexpected IMAP command");
                    assert!(
                        command.contains(&expected),
                        "expected {expected}, got {command}"
                    );
                    if let Some(hook) = &mut self.hook {
                        hook(&command);
                    }
                    self.log.lock().unwrap().push(command.clone());
                    self.pending.extend(response.replace("{tag}", &tag).bytes());
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl imap::extensions::idle::SetReadTimeout for Script {
        fn set_read_timeout(&mut self, _: Option<std::time::Duration>) -> imap::error::Result<()> {
            Ok(())
        }
    }
    type TestSession = imap::Session<TracedStream<Script>>;
    pub(crate) fn session(
        steps: Vec<Step>,
        hook: Option<Hook>,
    ) -> (TestSession, Trace, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(vec![]));
        let mut all = VecDeque::from([(String::from("LOGIN"), String::from("{tag} OK login\r\n"))]);
        all.extend(steps);
        let script = Script {
            idle_tag: None,
            pending: b"* OK ready\r\n".to_vec().into(),
            outgoing: vec![],
            steps: all,
            log: log.clone(),
            hook,
        };
        let (stream, trace) = TracedStream::new(script);
        let session = imap::Client::new(stream)
            .login("test@example.test", "fixture-secret")
            .map_err(|(error, _)| error)
            .unwrap();
        (session, trace, log)
    }
    pub(crate) fn step(expected: &str, response: &str) -> Step {
        (expected.into(), response.into())
    }
    fn capability() -> Step {
        step(
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n{tag} OK capabilities\r\n",
        )
    }
    fn select() -> Step {
        step("SELECT","* 1 EXISTS\r\n* OK [UIDVALIDITY 10] valid\r\n* OK [UIDNEXT 2] next\r\n{tag} OK selected\r\n")
    }
    fn fetch(flags: &str) -> Step {
        step(
            "UID FETCH 1",
            &format!("* 1 FETCH (UID 1 FLAGS ({flags}))\r\n{{tag}} OK flags\r\n"),
        )
    }
    fn db() -> SharedDb {
        Arc::new(Mutex::new(message_changes::tests::fixture()))
    }
    fn edit(db: &SharedDb, patch: message_changes::Patch) {
        message_changes::update(&mut db.lock().unwrap(), "acc", "m", &patch).unwrap();
    }
    fn statuses(db: &SharedDb) -> Vec<message_changes::ChangeStatus> {
        message_changes::statuses(&db.lock().unwrap(), "m").unwrap()
    }
    #[test]
    fn oversized_move_response_cannot_acknowledge_copyuid_or_repeat_the_move() {
        let db = db();
        edit(
            &db,
            message_changes::Patch {
                archive: Some(true),
                ..Default::default()
            },
        );
        let (mut s, t, log) = session(
            vec![
                capability(),
                select(),
                fetch(""),
                step(
                    "UID MOVE",
                    &format!(
                        "* OK [COPYUID 20 1 99] mapped\r\n* OK {}",
                        "x".repeat(1024 * 1024)
                    ),
                ),
            ],
            None,
        );
        assert!(flush(&db, "acc", &mut s, Some("Archive"), Some(&t)).is_err());
        assert_eq!(statuses(&db)[0].state, "unknown");
        assert_eq!(
            crate::mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
                .unwrap()
                .remote_id
                .as_deref(),
            Some("imap:inbox:10:1")
        );
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|line| line.contains("UID MOVE"))
                .count(),
            1
        );
        let (mut s, t, log) = session(vec![], None);
        flush(&db, "acc", &mut s, Some("Archive"), Some(&t)).unwrap();
        assert!(!log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("UID MOVE")));
    }
    #[test]
    fn disconnected_flags_are_durable_and_retry_only_the_selected_flag() {
        let db = db();
        edit(
            &db,
            message_changes::Patch {
                starred: Some(true),
                ..Default::default()
            },
        );
        let (mut s, t, _) = session(
            vec![
                capability(),
                select(),
                fetch("\\Answered"),
                step("UID STORE 1 +FLAGS.SILENT (\\Flagged)", ""),
            ],
            None,
        );
        assert_eq!(
            flush(&db, "acc", &mut s, Some("Archive"), Some(&t))
                .unwrap_err()
                .code,
            "network"
        );
        assert_eq!(statuses(&db)[0].state, "failed");
        let (mut s, t, log) = session(
            vec![
                capability(),
                select(),
                fetch("\\Answered"),
                step(
                    "UID STORE 1 +FLAGS.SILENT (\\Flagged)",
                    "{tag} OK stored\r\n",
                ),
                fetch("\\Answered \\Flagged"),
            ],
            None,
        );
        assert!(flush(&db, "acc", &mut s, Some("Archive"), Some(&t))
            .unwrap()
            .is_empty());
        assert!(statuses(&db).is_empty());
        let commands = log.lock().unwrap().join("");
        assert!(!commands.contains("EXPUNGE"));
        assert!(!commands.contains("FLAGS.SILENT (\\Answered"));
    }
    #[test]
    fn unread_uses_seen_removal_and_missing_uid_never_sends_store() {
        let db = db();
        edit(
            &db,
            message_changes::Patch {
                unread: Some(true),
                ..Default::default()
            },
        );
        let (mut s, t, _) = session(
            vec![
                capability(),
                select(),
                fetch("\\Seen"),
                step("UID STORE 1 -FLAGS.SILENT (\\Seen)", "{tag} OK stored\r\n"),
                fetch(""),
            ],
            None,
        );
        flush(&db, "acc", &mut s, None, Some(&t)).unwrap();
        assert!(statuses(&db).is_empty());
        edit(
            &db,
            message_changes::Patch {
                starred: Some(true),
                ..Default::default()
            },
        );
        let (mut s, t, log) = session(
            vec![
                capability(),
                select(),
                step("UID FETCH 1", "{tag} OK no such uid\r\n"),
            ],
            None,
        );
        assert!(!flush(&db, "acc", &mut s, None, Some(&t))
            .unwrap()
            .is_empty());
        assert_eq!(statuses(&db)[0].state, "conflict");
        assert!(!log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("STORE")));
    }
    #[test]
    fn newer_edit_during_store_is_not_acknowledged_and_db_is_unlocked() {
        let db = db();
        edit(
            &db,
            message_changes::Patch {
                starred: Some(true),
                ..Default::default()
            },
        );
        let other = db.clone();
        let hook: Hook = Box::new(move |command| {
            if command.contains("UID STORE") {
                let mut conn = other
                    .try_lock()
                    .expect("network I/O must not hold DB guard");
                message_changes::update(
                    &mut conn,
                    "acc",
                    "m",
                    &message_changes::Patch {
                        starred: Some(false),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        });
        let (mut s, t, _) = session(
            vec![
                capability(),
                select(),
                fetch(""),
                step("UID STORE", "{tag} OK stored\r\n"),
                fetch("\\Flagged"),
            ],
            Some(hook),
        );
        flush(&db, "acc", &mut s, None, Some(&t)).unwrap();
        let conn = db.lock().unwrap();
        let saved = crate::mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!saved.starred);
        assert_eq!(saved.changes[0].state, "queued");
        assert!(!message_changes::pending(&conn, "acc").unwrap()[0].value);
    }
    #[test]
    fn move_uses_tagged_copyuid_and_keeps_stable_id_and_new_pending_flags() {
        let db = db();
        edit(
            &db,
            message_changes::Patch {
                archive: Some(true),
                ..Default::default()
            },
        );
        let other = db.clone();
        let hook: Hook = Box::new(move |command| {
            if command.contains("UID MOVE") {
                let mut conn = other.try_lock().unwrap();
                message_changes::update(
                    &mut conn,
                    "acc",
                    "m",
                    &message_changes::Patch {
                        starred: Some(true),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        });
        let (mut s, t, log) = session(
            vec![
                capability(),
                select(),
                fetch(""),
                step(
                    "UID MOVE 1 \"Archive\"",
                    "{tag} OK [COPYUID 20 1 7] moved\r\n",
                ),
            ],
            Some(hook),
        );
        flush(&db, "acc", &mut s, Some("Archive"), Some(&t)).unwrap();
        let conn = db.lock().unwrap();
        let saved = crate::mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert_eq!(saved.mailbox, "archive");
        assert_eq!(saved.remote_id.as_deref(), Some("imap:archive:20:7"));
        assert_eq!(saved.id, "m");
        assert!(saved.starred);
        let pending = message_changes::pending(&conn, "acc").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].folder, "Archive");
        assert_eq!(pending[0].validity, 20);
        assert_eq!(pending[0].uid, 7);
        assert!(!log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("EXPUNGE") || line.contains("COPY ")));
    }
    #[test]
    fn ambiguous_move_is_never_retried_even_after_restart_recovery() {
        for response in [
            "",
            "{tag} OK moved without mapping\r\n",
            "{tag} NO outcome uncertain\r\n",
        ] {
            let db = db();
            edit(
                &db,
                message_changes::Patch {
                    archive: Some(true),
                    ..Default::default()
                },
            );
            let (mut s, t, _) = session(
                vec![
                    capability(),
                    select(),
                    fetch(""),
                    step("UID MOVE", response),
                ],
                None,
            );
            let _ = flush(&db, "acc", &mut s, Some("Archive"), Some(&t));
            assert_eq!(statuses(&db)[0].state, "unknown");
            message_changes::recover(&mut db.lock().unwrap()).unwrap();
            let (mut s, t, log) = session(vec![], None);
            flush(&db, "acc", &mut s, Some("Archive"), Some(&t)).unwrap();
            assert_eq!(log.lock().unwrap().len(), 1);
            assert_eq!(
                crate::mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
                    .unwrap()
                    .mailbox,
                "archive"
            );
        }
    }
    #[test]
    fn unsupported_move_uidvalidity_change_and_account_change_send_no_mutation() {
        let db = db();
        edit(
            &db,
            message_changes::Patch {
                archive: Some(true),
                ..Default::default()
            },
        );
        let (mut s, t, log) = session(
            vec![step(
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1\r\n{tag} OK capability\r\n",
            )],
            None,
        );
        flush(&db, "acc", &mut s, Some("Archive"), Some(&t)).unwrap();
        assert_eq!(statuses(&db)[0].state, "conflict");
        assert_eq!(log.lock().unwrap().len(), 2);
        edit(
            &db,
            message_changes::Patch {
                starred: Some(true),
                ..Default::default()
            },
        );
        let (mut s, t, _) = session(
            vec![
                capability(),
                step(
                    "SELECT",
                    "* OK [UIDVALIDITY 99] valid\r\n{tag} OK selected\r\n",
                ),
            ],
            None,
        );
        flush(&db, "acc", &mut s, None, Some(&t)).unwrap();
        assert!(statuses(&db).iter().all(|c| c.state == "conflict"));
        edit(
            &db,
            message_changes::Patch {
                starred: Some(false),
                ..Default::default()
            },
        );
        db.lock()
            .unwrap()
            .execute("UPDATE accounts SET email='new@example.test'", [])
            .unwrap();
        let (mut s, t, log) = session(vec![capability()], None);
        flush(&db, "acc", &mut s, None, Some(&t)).unwrap();
        assert_eq!(log.lock().unwrap().len(), 2);
    }
    #[test]
    fn copyuid_parser_skips_body_literals_and_rejects_conflicting_or_range_mapping() {
        let mut trace = MoveTrace::default();
        trace.arm(1);
        trace.sent(b"a8 UID MOVE 1 Archive\r\n");
        let fake = "* OK [COPYUID 99 1 9] forged\r\n";
        trace.received(
            format!(
                "* 1 FETCH (BODY[] {{{}}}\r\n{fake})\r\na8 OK done\r\n",
                fake.len()
            )
            .as_bytes(),
        );
        assert!(trace.mapping.is_none());
        for chunk in b"a8 OK [COPYUID 20 1:1 7:7] moved\r\n".chunks(3) {
            trace.received(chunk);
        }
        assert_eq!(trace.mapping, Some((20, 7)));
        trace.received(b"* OK [COPYUID 20 1 8] other\r\n");
        assert!(trace.invalid);
        trace.arm(1);
        trace.sent(b"a9 UID MOVE 1 Archive\r\n");
        trace.received(b"a9 OK [COPYUID 20 1:2 7:8] moved\r\n");
        assert!(trace.invalid);
    }
}
