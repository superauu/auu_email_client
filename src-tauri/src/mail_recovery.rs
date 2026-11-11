//! Explicit, read-only remote inspection followed by a separately confirmed recovery.
//! Requests share the account sync worker. Never hold a database guard across IMAP I/O.
use crate::{
    error::AppError,
    mail_store::{self, MessageRow},
    mail_sync::{self, SharedDb},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

#[path = "mail_recovery_scan.rs"]
mod scan;
#[path = "mail_recovery_segments.rs"]
mod segments;
#[cfg(test)]
#[path = "mail_recovery_tests.rs"]
mod tests;
const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 10 * 1024 * 1024;
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Check,
    Confirm,
    RetryArchive,
    Continue,
    AutoContinue,
    Fingerprint,
    ConfirmFingerprint,
}
impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Confirm => "confirm",
            Self::RetryArchive => "retry-archive",
            Self::Continue | Self::AutoContinue => "continue",
            Self::Fingerprint => "check",
            Self::ConfirmFingerprint => "confirm",
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub auto_continue: bool,
    pub state: String,
    pub detail: String,
    pub location: Option<Location>,
    pub evidence: Option<String>,
    pub progress: Option<Progress>,
    pub cross_uid_validity: bool,
    pub fingerprint: Option<FingerprintStatus>,
    pub operation: String,
}
#[derive(Debug, Serialize)]
pub struct FingerprintStatus {
    pub eligible: bool,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SegmentProgress {
    pub completed: usize,
    pub total: usize,
    pub phase: String,
}
#[derive(Debug, Serialize)]
pub struct Progress {
    pub checked: usize,
    pub total: usize,
    pub matches: usize,
    #[serde(skip)]
    cross_uid_validity: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments: Option<SegmentProgress>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub folder: String,
    pub kind: String,
    pub uid_validity: u32,
    pub uid: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Proof {
    location: Location,
    hash: String,
    header_id: Option<String>,
    result: String,
    strength: String,
    target: Option<(String, u32)>,
    #[serde(default)]
    scan_guard: Option<String>,
    #[serde(default)]
    cross_uid_validity: bool,
}
enum Inspection {
    Complete(Proof),
    Page(scan::Checkpoint),
    Segment(segments::Checkpoint),
}
enum PreparedInspection {
    Complete(Proof),
    Scan { json: String, progress: Progress },
}
impl Inspection {
    // Serialize large candidate lists before acquiring the database mutex.
    fn prepare(self) -> Result<PreparedInspection, AppError> {
        let (json, progress) = match self {
            Self::Complete(proof) => return Ok(PreparedInspection::Complete(proof)),
            Self::Page(checkpoint) => (serde_json::to_string(&checkpoint), checkpoint.progress()),
            Self::Segment(checkpoint) => {
                (serde_json::to_string(&checkpoint), checkpoint.progress())
            }
        };
        Ok(PreparedInspection::Scan {
            json: json.map_err(|_| AppError::storage("无法保存核对检查点"))?,
            progress,
        })
    }
}
struct Snapshot {
    auto_continue: bool,
    account: String,
    id: String,
    generation: String,
    revision: i64,
    action: String,
    previous: Option<Proof>,
    cached: MessageRow,
    source: Location,
    archive: Option<(String, String)>,
    scan_json: Option<String>,
    operation: String,
    fingerprint_anchor: bool,
}
fn fingerprint_status(conn: &Connection, id: &str) -> Result<Option<FingerprintStatus>, AppError> {
    type Anchor = (
        bool,
        bool,
        Option<String>,
        Option<String>,
        Option<u32>,
        Option<u32>,
        String,
        String,
    );
    let row:Option<Anchor>=conn.query_row("SELECT m.content_hash_verified,m.locator_verified,m.remote_generation,m.remote_folder,m.remote_uid_validity,m.remote_uid,COALESCE(m.original_remote_id,m.remote_id),a.sync_generation FROM messages m JOIN accounts a ON a.id=m.account_id WHERE m.id=?1 AND a.provider='imap' AND COALESCE(json_extract(CASE WHEN json_valid(a.profile_json) THEN a.profile_json ELSE '{}' END,'$.status'),'offline')!='demo'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(storage)?;
    Ok(row.and_then(
        |(verified, locator, origin, folder, validity, uid, remote, generation)| {
            if verified {
                return None;
            }
            let parts: Vec<_> = remote.split(':').collect();
            let tuple = parts.len() == 4
                && parts[0] == "imap"
                && crate::imap_settings::valid_scope(parts[1], folder.as_deref())
                && parts[2].parse::<u32>().ok() == validity
                && parts[3].parse::<u32>().ok() == uid;
            let eligible = tuple
                && locator
                && origin.as_deref() == Some(&generation)
                && folder.is_some()
                && validity.is_some_and(|n| n > 0)
                && uid.is_some_and(|n| n > 0);
            Some(FingerprintStatus {
                eligible,
                reason: if eligible {
                    "可从可信原定位重新读取完整原文，核对后明确补齐指纹。"
                } else {
                    "缺少可信原账号 / 文件夹 / UID 定位，不能把相似正文升级为历史原文指纹。"
                }
                .into(),
            })
        },
    ))
}
pub fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
fn storage(_: rusqlite::Error) -> AppError {
    AppError::storage("邮件恢复数据操作失败")
}
fn unresolved(message: &str) -> AppError {
    AppError::conflict(message)
}

pub fn status(conn: &Connection, id: &str) -> Result<Option<Status>, AppError> {
    let fingerprint = fingerprint_status(conn, id)?;
    type StatusRecord = (String, String, Option<String>, i64, i64, String, String);
    let row:Option<StatusRecord>=conn.query_row("SELECT r.state,r.detail,r.proof_json,r.revision,m.state_revision,r.generation,a.sync_generation FROM message_recovery r JOIN messages m ON m.id=r.message_id JOIN accounts a ON a.id=r.account_id WHERE r.message_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(storage)?;
    if let Some((mut state, mut detail, json, revision, current, origin, generation)) = row {
        let new_conflict:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM message_changes WHERE message_id=?1 AND state IN ('unknown','conflict'))",[id],|r|r.get(0)).map_err(storage)?;
        if state == "restored" && (new_conflict || origin != generation) {
            state = "available".into();
            detail = "邮件出现新的远端冲突，请重新核对。".into();
        }
        let proof = json.and_then(|json| serde_json::from_str::<Proof>(&json).ok());
        let progress = conn
            .query_row(
                    "SELECT checked,total,matches,cross_uid_validity FROM recovery_scan WHERE message_id=?1",
                [id],
                |r| {
                    Ok(Progress {
                        checked: r.get::<_, i64>(0)? as usize,
                        total: r.get::<_, i64>(1)? as usize,
                        matches: r.get::<_, u32>(2)? as usize,
                        cross_uid_validity: r.get(3)?,
                        segments: None,
                    })
                },
            )
            .optional()
            .map_err(storage)?;
        let mut progress = progress;
        if let Some(progress) = &mut progress {
            let json: Option<String> = conn
                .query_row(
                    "SELECT segment_progress_json FROM recovery_scan WHERE message_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            progress.segments = json
                .map(|text| {
                    serde_json::from_str(&text).map_err(|_| AppError::storage("分段进度无效"))
                })
                .transpose()?;
        }
        let operation: String = conn
            .query_row(
                "SELECT operation FROM message_recovery WHERE message_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if state == "located" && progress.is_some() {
            state = "scanning".into();
        }
        if (matches!(state.as_str(), "located" | "source-present" | "scanning")
            || (state == "failed" && progress.is_some()))
            && (revision != current || origin != generation)
        {
            state = "stale".into();
            detail = "核对结果已过期，请重新核对远端。".into();
        }
        return Ok(Some(Status {
            auto_continue: conn
                .query_row(
                    "SELECT auto_continue FROM message_recovery WHERE message_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(storage)?,
            state,
            detail,
            location: if progress.is_none() {
                proof.as_ref().map(|p| p.location.clone())
            } else {
                None
            },
            cross_uid_validity: proof.as_ref().is_some_and(|p| p.cross_uid_validity)
                || progress.as_ref().is_some_and(|p| p.cross_uid_validity),
            evidence: proof.map(|p| p.strength),
            progress,
            fingerprint,
            operation,
        }));
    }
    let eligible:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN accounts a ON a.id=m.account_id WHERE m.id=?1 AND a.provider='imap' AND COALESCE(json_extract(CASE WHEN json_valid(a.profile_json) THEN a.profile_json ELSE '{}' END,'$.status'),'offline')!='demo' AND COALESCE(m.original_remote_id,m.remote_id) LIKE 'imap:%' AND (m.remote_missing=1 OR m.remote_generation IS NULL OR m.remote_generation!=a.sync_generation OR m.remote_uid IS NULL OR EXISTS(SELECT 1 FROM message_changes c WHERE c.message_id=m.id AND c.state IN ('conflict','unknown'))))",[id],|r|r.get(0)).map_err(storage)?;
    Ok((eligible || fingerprint.is_some()).then(|| Status {
        auto_continue: false,
        state: "available".into(),
        detail: "远端定位需要核对；本机邮件和修改会保留。".into(),
        location: None,
        evidence: None,
        progress: None,
        cross_uid_validity: false,
        fingerprint,
        operation: "recovery".into(),
    }))
}

pub fn request(
    conn: &mut Connection,
    account: &str,
    id: &str,
    revision: i64,
    action: Action,
) -> Result<MessageRow, AppError> {
    let tx = conn.transaction().map_err(storage)?;
    let message = mail_store::message_by_id(&tx, account, id)?;
    if message.state_revision != revision {
        return Err(unresolved("邮件已改变，请刷新后重新核对。"));
    }
    let generation:String=tx.query_row("SELECT sync_generation FROM accounts WHERE id=?1 AND provider='imap' AND COALESCE(json_extract(CASE WHEN json_valid(profile_json) THEN profile_json ELSE '{}' END,'$.status'),'offline')!='demo'",[account],|r|r.get(0)).map_err(|_|AppError::invalid("只有真实 IMAP 缓存可核对远端"))?;
    let prior = status(&tx, id)?.ok_or_else(|| AppError::invalid("此邮件不需要定位恢复"))?;
    if matches!(prior.state.as_str(), "queued" | "checking")
        || message.changes.iter().any(|c| c.state == "applying")
    {
        return Err(unresolved("核对或回写正在进行，请等待结果。"));
    }
    if matches!(action, Action::Fingerprint | Action::ConfirmFingerprint)
        && !prior.fingerprint.as_ref().is_some_and(|f| f.eligible)
    {
        return Err(unresolved("缺少可信原定位，无法安全补齐历史原文指纹。"));
    }
    if matches!(action, Action::Confirm | Action::ConfirmFingerprint) && prior.state != "located"
        || matches!(action, Action::RetryArchive) && prior.state != "source-present"
        || matches!(action, Action::Continue | Action::AutoContinue)
            && !(prior.state == "scanning" || (prior.state == "failed" && prior.progress.is_some()))
    {
        return Err(unresolved("请先重新核对远端，再确认对应的恢复操作。"));
    }
    if matches!(action, Action::Confirm) && prior.operation != "recovery"
        || matches!(action, Action::ConfirmFingerprint) && prior.operation != "fingerprint"
    {
        return Err(unresolved("核对操作类型已改变，请重新核对。"));
    }
    if matches!(action, Action::Continue | Action::AutoContinue) {
        if matches!(action, Action::AutoContinue) {
            tx.execute(
                "UPDATE message_recovery SET auto_continue=1 WHERE account_id=?1 AND message_id=?2",
                params![account, id],
            )
            .map_err(storage)?;
        }
        tx.execute("UPDATE message_recovery SET state='queued',revision=?1,detail='下一页核对已登记，等待账号同步。',updated_at=?2 WHERE account_id=?3 AND message_id=?4",params![revision+1,crate::now_utc(),account,id]).map_err(storage)?;
    } else {
        if matches!(action, Action::Check | Action::Fingerprint) {
            tx.execute(
                "UPDATE message_recovery SET auto_continue=0 WHERE account_id=?1 AND message_id=?2",
                params![account, id],
            )
            .map_err(storage)?;
        }
        tx.execute(
            "DELETE FROM recovery_scan WHERE account_id=?1 AND message_id=?2",
            params![account, id],
        )
        .map_err(storage)?;
        tx.execute("INSERT INTO message_recovery(account_id,message_id,generation,revision,action,state,detail,proof_json,updated_at) VALUES (?1,?2,?3,?4,?5,'queued','已登记核对请求，等待账号同步。',NULL,?6) ON CONFLICT(account_id,message_id) DO UPDATE SET generation=excluded.generation,revision=excluded.revision,action=excluded.action,state='queued',detail=excluded.detail,proof_json=CASE WHEN excluded.action='check' THEN NULL ELSE message_recovery.proof_json END,updated_at=excluded.updated_at",params![account,id,generation,revision+1,action.name(),crate::now_utc()]).map_err(storage)?;
        tx.execute(
            "UPDATE message_recovery SET operation=?1 WHERE account_id=?2 AND message_id=?3",
            params![
                if matches!(action, Action::Fingerprint | Action::ConfirmFingerprint) {
                    "fingerprint"
                } else {
                    "recovery"
                },
                account,
                id
            ],
        )
        .map_err(storage)?;
    }
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1",
        [id],
    )
    .map_err(storage)?;
    let message = mail_store::message_by_id(&tx, account, id)?;
    tx.commit().map_err(storage)?;
    Ok(message)
}
pub fn recover(conn: &mut Connection) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(storage)?;
    tx.execute("DELETE FROM recovery_scan WHERE message_id IN (SELECT message_id FROM message_recovery WHERE state='checking')",[]).map_err(storage)?;
    tx.execute("UPDATE messages SET state_revision=state_revision+1 WHERE id IN (SELECT message_id FROM message_recovery WHERE state='checking')",[]).map_err(storage)?;
    tx.execute("UPDATE message_recovery SET state='queued',action='check',proof_json=NULL,detail='上次核对中断，下次手动同步重新检查。',revision=(SELECT state_revision FROM messages WHERE id=message_id) WHERE state='checking'",[]).map_err(storage)?;
    tx.commit().map_err(storage)
}
fn source(conn: &Connection, cached: &MessageRow) -> Result<Location, AppError> {
    let parts: Vec<_> = cached
        .remote_id
        .as_deref()
        .unwrap_or("")
        .split(':')
        .collect();
    if parts.len() != 4 || parts[0] != "imap" {
        return Err(unresolved("旧缓存没有可核对的协议定位。"));
    }
    let number = |text: &str| {
        text.parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .filter(|_| text.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| unresolved("旧缓存 UID 定位无效。"))
    };
    let (validity, uid) = (number(parts[2])?, number(parts[3])?);
    let stored: Option<String> = conn
        .query_row(
            "SELECT remote_folder FROM messages WHERE id=?1",
            [&cached.id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if !crate::imap_settings::valid_scope(parts[1], stored.as_deref()) {
        return Err(unresolved("远端文件夹与协议作用域不一致，不能重新绑定。"));
    }
    let folder = if let Some(folder) = stored {
        folder
    } else {
        let mut stmt=conn.prepare("SELECT remote_id FROM folders WHERE account_id=?1 AND kind=?2 ORDER BY remote_id LIMIT 2").map_err(storage)?;
        let folders = stmt
            .query_map(params![cached.account_id, parts[1]], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        if folders.len() != 1 {
            return Err(unresolved("原文件夹映射缺失或存在多个候选，不能猜测。"));
        }
        folders[0].clone()
    };
    Ok(Location {
        folder,
        kind: parts[1].into(),
        uid_validity: validity,
        uid,
    })
}
fn begin(conn: &mut Connection, account: &str, id: &str) -> Result<Snapshot, AppError> {
    let tx = conn.transaction().map_err(storage)?;
    let (generation,revision,action,previous):(String,i64,String,Option<String>)=tx.query_row("SELECT generation,revision,action,proof_json FROM message_recovery WHERE account_id=?1 AND message_id=?2 AND state='queued'",params![account,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(storage)?;
    let cached = mail_store::message_by_id(&tx, account, id)?;
    let current: String = tx
        .query_row(
            "SELECT sync_generation FROM accounts WHERE id=?1",
            [account],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if current != generation || cached.state_revision != revision {
        return Err(unresolved("账号或本机邮件已改变，请重新核对。"));
    }
    let source = source(&tx, &cached)?;
    let operation: String = tx
        .query_row(
            "SELECT operation FROM message_recovery WHERE account_id=?1 AND message_id=?2",
            params![account, id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let auto_continue: bool = tx
        .query_row(
            "SELECT auto_continue FROM message_recovery WHERE account_id=?1 AND message_id=?2",
            params![account, id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let fingerprint_anchor =
        cached.locator_verified && fingerprint_status(&tx, id)?.is_some_and(|f| f.eligible);
    let archive:Option<(String,Option<String>,String)>=tx.query_row("SELECT state,target_folder,generation FROM message_changes WHERE message_id=?1 AND field='archive' AND state='unknown'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
    let archive = if let Some((_, target, origin)) = archive {
        if origin != generation {
            return Err(unresolved("归档发生后账号连接已改变，不能恢复旧移动结果。"));
        }
        Some((
            target.ok_or_else(|| unresolved("归档目标未保存，不能猜测移动结果。"))?,
            origin,
        ))
    } else {
        None
    };
    let scan_json = tx
        .query_row(
            "SELECT checkpoint_json FROM recovery_scan WHERE account_id=?1 AND message_id=?2",
            params![account, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    tx.execute("UPDATE message_recovery SET state='checking',revision=revision+1,detail='正在只读核对远端定位。' WHERE account_id=?1 AND message_id=?2",params![account,id]).map_err(storage)?;
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1",
        [id],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok(Snapshot {
        auto_continue,
        account: account.into(),
        id: id.into(),
        generation,
        revision: revision + 1,
        action,
        previous: previous.and_then(|text| serde_json::from_str(&text).ok()),
        cached,
        source,
        archive,
        scan_json,
        operation,
        fingerprint_anchor,
    })
}

fn examine<S: Read + Write>(session: &mut imap::Session<S>, folder: &str) -> Result<u32, AppError> {
    session
        .examine(folder)
        .map_err(|error| crate::imap_limits::network(error, "无法只读打开核对文件夹"))?
        .uid_validity
        .filter(|n| *n > 0)
        .ok_or_else(|| unresolved("核对文件夹缺少 UIDVALIDITY。"))
}
fn trusted_hash(cached: &MessageRow) -> bool {
    cached.content_hash_verified
        && cached
            .content_hash
            .as_ref()
            .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn fingerprint<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    names: &[String],
) -> Result<Inspection, AppError> {
    if !s.fingerprint_anchor || !names.contains(&s.source.folder) {
        return Err(unresolved("没有可信原定位，不能安全补齐指纹。"));
    }
    if examine(session, &s.source.folder)? != s.source.uid_validity {
        return Err(unresolved(
            "原 UIDVALIDITY 已改变，不能用新候选补齐历史指纹。",
        ));
    }
    let mut total = 0;
    let first = fetch(session, s.source.uid, &s.cached, &mut total)?
        .filter(|mail| mail.matches)
        .ok_or_else(|| unresolved("原 UID 缺失或缓存内容不一致，未补齐指纹。"))?;
    if examine(session, &s.source.folder)? != s.source.uid_validity {
        return Err(unresolved("补齐核对期间 UIDVALIDITY 已改变。"));
    }
    let second = fetch(session, s.source.uid, &s.cached, &mut total)?
        .filter(|mail| mail.matches && mail.hash == first.hash)
        .ok_or_else(|| unresolved("两次完整原文读取不一致，未补齐指纹。"))?;
    Ok(Inspection::Complete(Proof {
        location: s.source.clone(),
        hash: second.hash,
        header_id: second.header_id,
        result: "fingerprint".into(),
        strength: "raw-sha256".into(),
        target: None,
        scan_guard: None,
        cross_uid_validity: false,
    }))
}
struct Remote {
    hash: String,
    header_id: Option<String>,
    matches: bool,
}
fn fetch<S: Read + Write>(
    session: &mut imap::Session<S>,
    uid: u32,
    cached: &MessageRow,
    total: &mut usize,
) -> Result<Option<Remote>, AppError> {
    let sizes = session
        .uid_fetch(uid.to_string(), "(UID RFC822.SIZE)")
        .map_err(|error| crate::imap_limits::network(error, "核对邮件大小失败"))?;
    if sizes.is_empty() {
        return Ok(None);
    }
    if sizes.len() != 1 || sizes[0].uid != Some(uid) {
        return Err(unresolved("大小响应 UID 不匹配。"));
    }
    let size = sizes[0]
        .size
        .filter(|size| *size > 0 && *size <= MAX_MESSAGE_BYTES)
        .ok_or_else(|| unresolved("邮件大小缺失或超过 1 MiB 核对上限。"))?;
    *total += size as usize;
    if *total > MAX_TOTAL_BYTES {
        return Err(unresolved("本次核对正文超过 10 MiB 上限，未采用部分结果。"));
    }
    let values = session
        .uid_fetch(uid.to_string(), "(UID INTERNALDATE BODY.PEEK[])")
        .map_err(|error| crate::imap_limits::network(error, "核对邮件内容失败"))?;
    if values.len() != 1 || values[0].uid != Some(uid) {
        return Err(unresolved("正文响应缺失或 UID 不匹配。"));
    }
    let raw = values[0]
        .body()
        .ok_or_else(|| unresolved("核对响应缺少正文。"))?;
    if raw.len() != size as usize {
        return Err(unresolved("邮件正文与声明大小不一致。"));
    }
    let parsed = mail_sync::parse_message(&values[0])?;
    Ok(Some(Remote {
        hash: parsed.content_hash.clone(),
        header_id: parsed.header_message_id.clone(),
        matches: mail_sync::matches_cached(&parsed, cached),
    }))
}
fn inspect<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    names: &[String],
) -> Result<Inspection, AppError> {
    if s.operation == "fingerprint" {
        return fingerprint(session, s, names);
    }
    if s.scan_json
        .as_ref()
        .is_some_and(|json| segments::is_checkpoint(json))
    {
        return segments::page(session, s, names);
    }
    if s.scan_json.is_some() {
        return scan::page(session, s, names);
    }
    if !names.contains(&s.source.folder) {
        return Err(unresolved("原文件夹不在当前服务器列表中。"));
    }
    if s.cached.remote_missing && s.archive.is_none() {
        if !trusted_hash(&s.cached) {
            return Err(unresolved(
                "原 UID 已消失，缺少可信完整原文，不能搜索其他文件夹绑定。",
            ));
        }
        return segments::page(session, s, names);
    }
    let source_mailbox = session
        .examine(&s.source.folder)
        .map_err(|error| crate::imap_limits::network(error, "无法只读打开原文件夹"))?;
    let source_validity = source_mailbox
        .uid_validity
        .filter(|n| *n > 0)
        .ok_or_else(|| unresolved("原文件夹缺少 UIDVALIDITY。"))?;
    if source_validity != s.source.uid_validity {
        if !trusted_hash(&s.cached) {
            return Err(unresolved(
                "UIDVALIDITY 已改变且缺少完整原文指纹，不能安全跨代重绑定。",
            ));
        }
        if segments::should_segment(&source_mailbox, s.cached.header_message_id.is_some()) {
            return segments::page(session, s, names);
        }
        return scan::page(session, s, names);
    }
    let mut total = 0;
    let original = fetch(session, s.source.uid, &s.cached, &mut total)?;
    if let Some((target, _)) = &s.archive {
        if target == &s.source.folder || !names.contains(target) {
            return Err(unresolved("保存的归档目标缺失或与源相同。"));
        }
        if original.as_ref().is_some_and(|mail| !mail.matches) {
            return Err(unresolved("源 UID 的邮件内容与本机不同。"));
        }
        let target_mailbox = session
            .examine(target)
            .map_err(|error| crate::imap_limits::network(error, "无法只读打开归档目标"))?;
        let validity = target_mailbox
            .uid_validity
            .filter(|n| *n > 0)
            .ok_or_else(|| unresolved("目标缺少 UIDVALIDITY。"))?;
        if segments::should_segment(&target_mailbox, s.cached.header_message_id.is_some()) {
            return segments::page(session, s, names);
        }
        let query = if let Some(id) = &s.cached.header_message_id {
            if id.len() > 998 || id.chars().any(char::is_control) {
                return Err(unresolved("缓存 Message-ID 不适合安全搜索。"));
            }
            format!(
                "HEADER Message-ID \"{}\"",
                id.replace('\\', "\\\\").replace('"', "\\\"")
            )
        } else {
            "ALL".into()
        };
        let uids = session
            .uid_search(query)
            .map_err(|error| crate::imap_limits::network(error, "归档目标候选搜索失败"))?;
        if uids.contains(&0) {
            return Err(unresolved("候选 UID 无效，未采用部分搜索结果。"));
        }
        if uids.len() > scan::PAGE_SIZE {
            return scan::page(session, s, names);
        }
        let mut matches = vec![];
        let mut uids: Vec<_> = uids.into_iter().collect();
        uids.sort_unstable();
        for uid in uids {
            let mail = fetch(session, uid, &s.cached, &mut total)?
                .ok_or_else(|| unresolved("搜索期间候选消失，请重新核对。"))?;
            if mail.matches {
                matches.push((uid, mail));
            }
        }
        // Recheck the source after the target scan; concurrent cross-client moves must not imply success.
        if examine(session, &s.source.folder)? != s.source.uid_validity {
            return Err(unresolved("核对期间源文件夹已改变。"));
        }
        let again = fetch(session, s.source.uid, &s.cached, &mut total)?;
        if original.as_ref().map(|m| &m.hash) != again.as_ref().map(|m| &m.hash) {
            return Err(unresolved("核对期间源邮件发生变化，请重新核对。"));
        }
        match (again, matches.len()) {
            (None, 1) => {
                let (uid, mail) = matches.remove(0);
                Ok(Inspection::Complete(Proof {
                    location: Location {
                        folder: target.clone(),
                        kind: "archive".into(),
                        uid_validity: validity,
                        uid,
                    },
                    hash: mail.hash,
                    header_id: mail.header_id,
                    result: "archived".into(),
                    strength: strength(&s.cached),
                    target: None,
                    scan_guard: None,
                    cross_uid_validity: false,
                }))
            }
            (Some(mail), 0) => Ok(Inspection::Complete(Proof {
                location: s.source.clone(),
                hash: mail.hash,
                header_id: mail.header_id,
                result: "source-present".into(),
                strength: strength(&s.cached),
                target: Some((target.clone(), validity)),
                scan_guard: None,
                cross_uid_validity: false,
            })),
            _ => Err(unresolved(
                "源和目标同时存在、匹配重复或均未找到；归档结果仍不确定。",
            )),
        }
    } else {
        let mail =
            original.ok_or_else(|| unresolved("原 UID 不存在，不能自动寻找其他邮件替代。"))?;
        if !mail.matches {
            return Err(unresolved("原 UID 的邮件内容与本机不同，未恢复定位。"));
        }
        Ok(Inspection::Complete(Proof {
            location: s.source.clone(),
            hash: mail.hash,
            header_id: mail.header_id,
            result: "legacy".into(),
            strength: strength(&s.cached),
            target: None,
            scan_guard: None,
            cross_uid_validity: false,
        }))
    }
}
fn strength(cached: &MessageRow) -> String {
    if cached.content_hash.is_some() {
        "raw-sha256"
    } else {
        "cached-fields"
    }
    .into()
}

pub fn process<S: Read + Write>(
    db: &SharedDb,
    account: &str,
    session: &mut imap::Session<S>,
    names: &[String],
) -> Result<Vec<String>, AppError> {
    let ids = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let mut stmt=conn.prepare("SELECT message_id FROM message_recovery WHERE account_id=?1 AND state='queued' ORDER BY updated_at,message_id LIMIT 20").map_err(storage)?;
        let result = stmt
            .query_map([account], |r| r.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        result
    };
    let mut warnings = vec![];
    for id in ids {
        let snapshot = {
            let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            begin(&mut conn, account, &id)
        };
        let result = match snapshot {
            Ok(s) => {
                let result = inspect(session, &s, names)
                    .and_then(Inspection::prepare)
                    .and_then(|result| {
                        crate::sync_control::check()?;
                        Ok(result)
                    });
                let interrupted = result
                    .as_ref()
                    .err()
                    .filter(|e| ["network", "cancelled"].contains(&e.code))
                    .cloned();
                let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
                if let Err(error) = finish(&mut conn, &s, result) {
                    finish(&mut conn, &s, Err(error.clone()))?;
                    return Err(error);
                }
                if let Some(status) = status(&conn, &id)? {
                    if matches!(status.state.as_str(), "unresolved" | "failed") {
                        warnings.push(status.detail);
                    }
                }
                if let Some(error) = interrupted {
                    return Err(error);
                }
                continue;
            }
            Err(error) => error,
        };
        let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let tx = conn.transaction().map_err(storage)?;
        tx.execute(
            "DELETE FROM recovery_scan WHERE account_id=?1 AND message_id=?2",
            params![account, id],
        )
        .map_err(storage)?;
        tx.execute("UPDATE messages SET state_revision=state_revision+1 WHERE id=?1 AND EXISTS(SELECT 1 FROM message_recovery WHERE message_id=?1 AND state='queued')",[&id]).map_err(storage)?;
        tx.execute("UPDATE message_recovery SET state='unresolved',detail=?1,proof_json=NULL,revision=(SELECT state_revision FROM messages WHERE id=?3) WHERE account_id=?2 AND message_id=?3 AND state='queued'",params![result.message,account,id]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        warnings.push(result.message);
    }
    Ok(warnings)
}

fn finish(
    conn: &mut Connection,
    s: &Snapshot,
    result: Result<PreparedInspection, AppError>,
) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(storage)?;
    let row:Option<(String,i64,String,String,i64)>=tx.query_row("SELECT r.state,r.revision,r.generation,a.sync_generation,m.state_revision FROM message_recovery r JOIN accounts a ON a.id=r.account_id JOIN messages m ON m.id=r.message_id WHERE r.account_id=?1 AND r.message_id=?2",params![s.account,s.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(storage)?;
    let Some((state, revision, origin, generation, current)) = row else {
        return Ok(());
    };
    if state != "checking" || revision != s.revision {
        return Ok(());
    }
    let result = if origin != s.generation || generation != s.generation || current != s.revision {
        Err(unresolved("核对期间账号或本机修改已改变，请重新核对。"))
    } else {
        result
    };
    let mut keep_scan = false;
    let (state, detail, proof) = match result {
        Ok(PreparedInspection::Scan { json, progress }) => {
            save_scan(&tx, s, &json, &progress)?;
            keep_scan = true;
            let detail = if let Some(segment) = &progress.segments {
                format!(
                    "分段{} {}/{}，已检查 {} 个候选；全段完成前不能确认绑定。",
                    if segment.phase == "collecting" {
                        "正文核对"
                    } else {
                        "集合复核"
                    },
                    segment.completed,
                    segment.total,
                    progress.checked
                )
            } else {
                format!(
                    "分页核对已检查 {} / {} 个候选，尚未完成；请继续下一页。",
                    progress.checked, progress.total
                )
            };
            (
                if s.auto_continue { "queued" } else { "located" },
                detail,
                s.previous.clone(),
            )
        }
        Err(error) => {
            keep_scan = ["network", "cancelled"].contains(&error.code) && s.scan_json.is_some();
            (
                if error.code == "network" && s.auto_continue {
                    "queued"
                } else if ["network", "cancelled"].contains(&error.code) {
                    "failed"
                } else {
                    "unresolved"
                },
                error.message,
                if keep_scan || (error.code == "network" && s.auto_continue) {
                    s.previous.clone()
                } else {
                    None
                },
            )
        }
        Ok(PreparedInspection::Complete(proof)) if s.action != "check" => {
            if s.previous.as_ref() != Some(&proof)
                || (s.action == "retry-archive") != (proof.result == "source-present")
            {
                (
                    "unresolved",
                    "远端候选已改变，旧核对结果未被应用，请重新核对。".into(),
                    None,
                )
            } else {
                // SQL failure rolls back the entire local merge; the caller can check again without a MOVE.
                apply(&tx, s, &proof)?;
                (
                    "restored",
                    if proof.result == "fingerprint" {
                        "可信完整原文指纹已补齐；本地修改与远端定位保持。"
                    } else if proof.result == "source-present" {
                        "已明确重新排队归档；下次手动同步尝试移动。"
                    } else {
                        "定位已恢复，本机修改保留；待回写字段下次手动同步处理。"
                    }
                    .into(),
                    Some(proof),
                )
            }
        }
        Ok(PreparedInspection::Complete(proof)) => {
            let state = if proof.result == "source-present" {
                "source-present"
            } else {
                "located"
            };
            let detail = if proof.result == "fingerprint" {
                "原定位未变且两次完整原文一致，请明确确认补齐指纹；不会重新绑定或回写邮件。"
            } else if proof.cross_uid_validity {
                "已跨 UIDVALIDITY 完整检查候选并匹配原文指纹；请确认恢复新定位，确认时将重新完整核对。"
            } else if proof.result == "source-present" {
                "原邮件仍在源文件夹，目标未找到匹配。可明确重新排队归档。"
            } else if proof.strength == "cached-fields" {
                "已找到内容字段相符的定位；旧缓存无完整原文指纹，请核对当前邮件后明确确认。"
            } else {
                "已核对原文指纹及远端定位，请确认恢复。"
            };
            (state, detail.into(), Some(proof))
        }
    };
    if !keep_scan {
        tx.execute(
            "DELETE FROM recovery_scan WHERE account_id=?1 AND message_id=?2",
            params![s.account, s.id],
        )
        .map_err(storage)?;
    }
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1",
        [&s.id],
    )
    .map_err(storage)?;
    tx.execute("UPDATE message_recovery SET state=?1,detail=?2,proof_json=?3,revision=?4,updated_at=?5 WHERE account_id=?6 AND message_id=?7",params![state,detail,proof.map(|p|serde_json::to_string(&p)).transpose().map_err(|_|AppError::storage("恢复证据序列化失败"))?,current+1,crate::now_utc(),s.account,s.id]).map_err(storage)?;
    tx.commit().map_err(storage)
}
fn save_scan(
    tx: &rusqlite::Transaction<'_>,
    s: &Snapshot,
    json: &str,
    progress: &Progress,
) -> Result<(), AppError> {
    let segments = progress
        .segments
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| AppError::storage("无法保存分段进度"))?;
    tx.execute("INSERT INTO recovery_scan(account_id,message_id,checkpoint_json,checked,total,matches,cross_uid_validity,segment_progress_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(account_id,message_id) DO UPDATE SET checkpoint_json=excluded.checkpoint_json,checked=excluded.checked,total=excluded.total,matches=excluded.matches,cross_uid_validity=excluded.cross_uid_validity,segment_progress_json=excluded.segment_progress_json",params![s.account,s.id,json,progress.checked as i64,progress.total as i64,progress.matches as i64,progress.cross_uid_validity,segments]).map_err(storage)?;
    Ok(())
}

fn apply(tx: &rusqlite::Transaction<'_>, s: &Snapshot, p: &Proof) -> Result<(), AppError> {
    if (p.cross_uid_validity || (p.result == "legacy" && p.location.folder != s.source.folder))
        && !trusted_hash(&s.cached)
    {
        return Err(unresolved("没有可信历史原文指纹，拒绝跨代绑定。"));
    }
    if p.result == "fingerprint" {
        if s.operation != "fingerprint" || !s.fingerprint_anchor || p.location != s.source {
            return Err(unresolved("原定位证据不足，不能补齐可信指纹。"));
        }
        tx.execute("UPDATE messages SET content_hash=?1,header_message_id=?2,content_hash_verified=1 WHERE account_id=?3 AND id=?4",params![p.hash,p.header_id,s.account,s.id]).map_err(storage)?;
        return Ok(());
    }
    let mut remote_id = format!(
        "imap:{}:{}:{}",
        p.location.kind, p.location.uid_validity, p.location.uid
    );
    let duplicates = {
        let mut stmt=tx.prepare("SELECT id,remote_id FROM messages WHERE account_id=?1 AND id!=?3 AND (remote_id=?2 OR (remote_folder=?4 AND remote_uid_validity=?5 AND remote_uid=?6 AND remote_generation=?7 AND locator_verified=1)) LIMIT 2").map_err(storage)?;
        let rows = stmt
            .query_map(
                params![
                    s.account,
                    remote_id,
                    s.id,
                    p.location.folder,
                    p.location.uid_validity,
                    p.location.uid,
                    s.generation
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    };
    if duplicates.len() > 1 {
        return Err(unresolved(
            "目标存在多个本机缓存，保留全部数据，不能选择其中一份。",
        ));
    }
    if let Some((id, existing_remote)) = duplicates.into_iter().next() {
        let cached = mail_store::message_by_id(tx, &s.account, &id)?;
        if cached
            .content_hash
            .as_ref()
            .is_some_and(|hash| hash != &p.hash)
            || (cached.content_hash.is_none()
                && (cached.subject != s.cached.subject
                    || cached.body_text != s.cached.body_text
                    || cached.from.email != s.cached.from.email
                    || cached.to != s.cached.to
                    || cached.received_at != s.cached.received_at))
        {
            return Err(unresolved("目标缓存的内容不同，保留两份，不能自动合并。"));
        }
        let edited:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM message_changes WHERE message_id=?1) OR EXISTS(SELECT 1 FROM message_recovery WHERE message_id=?1)",[&id],|r|r.get(0)).map_err(storage)?;
        if edited {
            return Err(unresolved("目标缓存也有本机修改，保留两份，不能自动合并。"));
        }
        remote_id = existing_remote;
        tx.execute("INSERT OR IGNORE INTO message_labels SELECT ?1,label_id FROM message_labels WHERE message_id=?2",params![s.id,id]).map_err(storage)?;
        tx.execute(
            "UPDATE mail_events SET message_id=?1 WHERE account_id=?2 AND message_id=?3",
            params![s.id, s.account, id],
        )
        .map_err(storage)?;
        tx.execute("UPDATE drafts SET reply_to_id=?1,revision=revision+1 WHERE account_id=?2 AND reply_to_id=?3",params![s.id,s.account,id]).map_err(storage)?;
        tx.execute(
            "DELETE FROM messages WHERE account_id=?1 AND id=?2",
            params![s.account, id],
        )
        .map_err(storage)?;
    }
    if p.result == "archived" {
        tx.execute("DELETE FROM message_changes WHERE message_id=?1 AND field='archive' AND state='unknown'",[&s.id]).map_err(storage)?;
    }
    tx.execute("UPDATE messages SET remote_id=?1,remote_folder=?2,remote_uid_validity=?3,remote_uid=?4,remote_generation=?5,content_hash=?6,header_message_id=?7,remote_missing=0,original_remote_id=NULL WHERE account_id=?8 AND id=?9",params![remote_id,p.location.folder,p.location.uid_validity,p.location.uid,s.generation,p.hash,p.header_id,s.account,s.id]).map_err(storage)?;
    tx.execute(
        "UPDATE messages SET content_hash_verified=?1,locator_verified=?2 WHERE id=?3",
        params![
            trusted_hash(&s.cached) && s.cached.content_hash.as_deref() == Some(&p.hash),
            trusted_hash(&s.cached) || (s.fingerprint_anchor && p.location == s.source),
            s.id
        ],
    )
    .map_err(storage)?;
    let folder = format!("{}:folder:{}", s.account, p.location.folder);
    tx.execute("INSERT INTO folders(id,account_id,remote_id,name,kind) VALUES (?1,?2,?3,?3,?4) ON CONFLICT(account_id,remote_id) DO NOTHING",params![folder,s.account,p.location.folder,p.location.kind]).map_err(storage)?;
    tx.execute(
        "DELETE FROM message_folders WHERE account_id=?1 AND message_id=?2",
        params![s.account, s.id],
    )
    .map_err(storage)?;
    tx.execute("INSERT INTO message_folders SELECT ?1,?2,id FROM folders WHERE account_id=?1 AND remote_id=?3",params![s.account,s.id,p.location.folder]).map_err(storage)?;
    tx.execute("UPDATE message_changes SET state='queued',error=NULL,generation=?1,source_folder=?2,source_validity=?3,source_uid=?4,revision=?5 WHERE account_id=?6 AND message_id=?7 AND state NOT IN ('local','unknown','applying')",params![s.generation,p.location.folder,p.location.uid_validity,p.location.uid,s.revision+1,s.account,s.id]).map_err(storage)?;
    if p.result == "source-present" {
        tx.execute("UPDATE message_changes SET state='queued',error=NULL,revision=?1,generation=?4,source_folder=?5,source_validity=?6,source_uid=?7 WHERE account_id=?2 AND message_id=?3 AND field='archive' AND state='unknown'",params![s.revision+1,s.account,s.id,s.generation,p.location.folder,p.location.uid_validity,p.location.uid]).map_err(storage)?;
    }
    Ok(())
}
