//! Durable, account-scoped local drafts. Saving never sends or uploads a message.
use crate::{
    compose_attach::{self, ComposeStaging},
    error::AppError,
    mail_store::{FromRow, MessageRow},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const LEGACY_SOURCE: &str = "auumail.demo.drafts.v1";
const CONFLICT: &str = "草稿已在其他窗口修改，请保留当前内容并重新打开草稿。";

/// 附件限制（TS 侧对称镜像）：每封邮件最多 20 个、原始字节合计 16 MiB。
pub(crate) const MAX_ATTACH_COUNT: usize = 20;
pub(crate) const MAX_ATTACH_TOTAL: usize = 16 * 1024 * 1024;

/// body_html 上限（TS 侧对称镜像）。200k → 2M：签名 / 内联图片以 data:image/* URI
/// 内嵌在正文 HTML 里（base64 约占 4/3 原始字节），发送时由 outbox 转 cid: 内联件；
/// 量级与收信侧单 HTML 上限（mail_html::MAX_HTML，2 MiB）对齐。
pub(crate) const MAX_BODY_HTML: usize = 2_000_000;

/// 撰写附件元数据；字节内容存 draft_attachment_blobs，前端与暂存区仅持有本结构。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftAttachment {
    pub id: String,
    pub name: String,
    pub size: usize,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    pub subject: String,
    pub body_text: String,
    /// 撰写编辑器产出的 HTML 正文（写入时服务端消毒）；None = 纯文本草稿。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    /// 撰写附件（有序元数据数组；旧草稿缺省为空）。
    #[serde(default)]
    pub attachments: Vec<DraftAttachment>,
    pub updated_at: String,
    #[serde(default)]
    pub revision: u32,
}

/// 草稿标识规则（保存校验、删除与附件暂存共用）。
pub(crate) fn valid_draft_id(id: &str) -> bool {
    id.starts_with("draft-")
        && id.len() > 6
        && id.len() <= 100
        && id[6..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn valid_attachment_id(id: &str) -> bool {
    id.starts_with("att-")
        && id.len() > 4
        && id.len() <= 100
        && id[4..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// 附件元数据校验（保存与发送登记共用；TS 侧 validateDraft 镜像同一组限制）。
pub(crate) fn validate_attachments(attachments: &[DraftAttachment]) -> Result<(), AppError> {
    if attachments.len() > MAX_ATTACH_COUNT {
        return Err(AppError::invalid("附件数量超出限制（每封邮件最多 20 个）"));
    }
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    for attachment in attachments {
        if !valid_attachment_id(&attachment.id)
            || !seen.insert(attachment.id.clone())
            || attachment.name.trim().is_empty()
            || attachment.name.encode_utf16().count() > 240
            || attachment.media_type.is_empty()
            || attachment.media_type.len() > 100
            || attachment.sha256.len() != 64
            || !attachment
                .sha256
                .bytes()
                .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
            || attachment.size > MAX_ATTACH_TOTAL
        {
            return Err(AppError::invalid("附件信息无效或超出长度限制"));
        }
        total += attachment.size;
        if total > MAX_ATTACH_TOTAL {
            return Err(AppError::invalid(
                "附件总大小超出限制（16 MiB），请移除部分附件后重试。",
            ));
        }
    }
    Ok(())
}

fn validate(draft: &Draft) -> Result<(), AppError> {
    if !valid_draft_id(&draft.id)
        || draft.account_id.is_empty()
        || draft.account_id.len() > 200
        || draft.to.encode_utf16().count() > 1000
        || draft.cc.encode_utf16().count() > 1000
        || draft.bcc.encode_utf16().count() > 1000
        || draft.subject.encode_utf16().count() > 1000
        || draft.body_text.encode_utf16().count() > 50_000
        || draft
            .body_html
            .as_ref()
            .is_some_and(|html| html.encode_utf16().count() > MAX_BODY_HTML)
        || draft.revision > 1_000_000_000
        || draft
            .reply_to_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 200)
    {
        return Err(AppError::invalid("草稿字段无效或超出长度限制"));
    }
    validate_attachments(&draft.attachments)
}

/// 附件落库（保存事务内）：暂存有字节则写入子表；暂存缺失时要求已有同 sha256 的行
/// （此前保存过）。列表外的旧行删除；事务回滚时暂存保留，重试仍可用。
fn save_attachments(
    conn: &Connection,
    draft: &Draft,
    staging: &ComposeStaging,
) -> Result<(), AppError> {
    let mut kept = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        kept.push(attachment.id.clone());
        let content = match compose_attach::staged_content(
            staging,
            &draft.id,
            &attachment.id,
            &attachment.sha256,
        ) {
            Some(content) => content,
            None => {
                let existing: Option<String> = conn
                    .query_row(
                        "SELECT sha256 FROM draft_attachment_blobs WHERE draft_id = ?1 AND attachment_id = ?2",
                        params![draft.id, attachment.id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|_| AppError::storage("无法读取附件内容"))?;
                match existing {
                    Some(sha256) if sha256 == attachment.sha256 => continue,
                    _ => {
                        return Err(AppError::invalid(format!(
                            "附件「{}」内容缺失，请重新添加该附件。",
                            attachment.name
                        )))
                    }
                }
            }
        };
        if content.len() != attachment.size {
            return Err(AppError::invalid(format!(
                "附件「{}」内容与元数据不一致，请重新添加。",
                attachment.name
            )));
        }
        conn.execute(
            "INSERT INTO draft_attachment_blobs (draft_id, attachment_id, sha256, content)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(draft_id, attachment_id) DO UPDATE SET sha256 = excluded.sha256, content = excluded.content",
            params![draft.id, attachment.id, attachment.sha256, content],
        )
        .map_err(|_| AppError::storage("附件未能写入本机，请保留当前内容后重试。"))?;
    }
    let mut statement = conn
        .prepare("SELECT attachment_id FROM draft_attachment_blobs WHERE draft_id = ?1")
        .map_err(|_| AppError::storage("无法读取附件列表"))?;
    let existing: Vec<String> = statement
        .query_map([&draft.id], |row| row.get(0))
        .map_err(|_| AppError::storage("无法读取附件列表"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("无法读取附件列表"))?;
    for id in existing {
        if !kept.contains(&id) {
            conn.execute(
                "DELETE FROM draft_attachment_blobs WHERE draft_id = ?1 AND attachment_id = ?2",
                params![draft.id, id],
            )
            .map_err(|_| AppError::storage("附件未能从本机移除，请重试。"))?;
        }
    }
    Ok(())
}

fn write(
    conn: &Connection,
    draft: &Draft,
    previous_account_id: &str,
    staging: &ComposeStaging,
) -> Result<Draft, AppError> {
    validate(draft)?;
    let account_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
            [&draft.account_id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法读取发件账号"))?;
    if !account_exists {
        return Err(AppError::invalid(
            "发件账号已不存在，请保留内容并选择其他账号。",
        ));
    }
    let existing: Option<(String, u32)> = conn
        .query_row(
            "SELECT account_id, revision FROM drafts WHERE id = ?1",
            [&draft.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取草稿版本"))?;
    match existing {
        Some((account, revision))
            if account == previous_account_id && revision == draft.revision => {}
        None if draft.revision == 0 => {}
        _ => return Err(AppError::conflict(CONFLICT)),
    }
    let in_outbox: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = ?1)",
            [&draft.id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法检查发送状态"))?;
    if in_outbox {
        return Err(AppError::conflict(
            "这封草稿已进入发送流程，请保留当前内容并另建草稿。",
        ));
    }
    if let Some(reply_id) = &draft.reply_to_id {
        let owner: Option<String> = conn
            .query_row(
                "SELECT account_id FROM messages WHERE id = ?1",
                [reply_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| AppError::storage("无法检查回复关联"))?;
        if owner.is_some_and(|account| account != draft.account_id) {
            return Err(AppError::invalid("回复草稿必须使用原邮件所属账号"));
        }
    }
    let mut saved = draft.clone();
    saved.revision += 1;
    saved.updated_at = crate::now_utc();
    // 撰写 HTML 是发给别人的内容：写入前服务端消毒（compose 专用配置，剥脚本与相对地址）。
    saved.body_html = saved
        .body_html
        .as_ref()
        .map(|html| crate::mail_html::sanitize_compose(html))
        .filter(|html| !html.trim().is_empty());
    let recipients = serde_json::to_string(&if saved.to.is_empty() {
        Vec::new()
    } else {
        vec![&saved.to]
    })
    .map_err(|_| AppError::invalid("收件人格式无效"))?;
    let attachments_json =
        serde_json::to_string(&saved.attachments).map_err(|_| AppError::invalid("附件信息无效"))?;
    conn.execute("INSERT INTO drafts (id, account_id, recipients_json, subject, body_text, body_html, reply_to_id, updated_at, revision, cc, bcc, attachments_json)
      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
      ON CONFLICT(id) DO UPDATE SET account_id = excluded.account_id, recipients_json = excluded.recipients_json,
      subject = excluded.subject, body_text = excluded.body_text, body_html = excluded.body_html, reply_to_id = excluded.reply_to_id,
      updated_at = excluded.updated_at, revision = excluded.revision, cc = excluded.cc, bcc = excluded.bcc, attachments_json = excluded.attachments_json",
      params![saved.id, saved.account_id, recipients, saved.subject, saved.body_text, saved.body_html.clone().unwrap_or_default(), saved.reply_to_id, saved.updated_at, saved.revision, saved.cc, saved.bcc, attachments_json])
      .map_err(|_| AppError::storage("草稿未能写入本机，请保留当前内容后重试。"))?;
    save_attachments(conn, &saved, staging)?;
    Ok(saved)
}

fn project(conn: &Connection, draft: Draft) -> Result<MessageRow, AppError> {
    let from = conn
        .query_row(
            "SELECT display_name, email FROM accounts WHERE id = ?1",
            [&draft.account_id],
            |row| {
                Ok(FromRow {
                    name: row.get(0)?,
                    email: row.get(1)?,
                })
            },
        )
        .map_err(|_| AppError::storage("无法读取草稿所属账号"))?;
    Ok(MessageRow {
        body_download: None,
        folder_ids: vec![],
        attachments: vec![],
        remote_missing: false,
        recovery: None,
        content_hash: None,
        locator_verified: false,
        content_hash_verified: false,
        header_message_id: None,
        header_references: None,
        state_revision: 0,
        changes: vec![],
        id: draft.id.clone(),
        account_id: draft.account_id.clone(),
        remote_id: None,
        thread_id: draft
            .reply_to_id
            .clone()
            .unwrap_or_else(|| draft.id.clone()),
        mailbox: "drafts".into(),
        from,
        to: if draft.to.is_empty() {
            Vec::new()
        } else {
            vec![draft.to.clone()]
        },
        // 草稿 cc 为逗号分隔字符串；仅用于界面投影（草稿不提供回复操作）。
        cc: draft
            .cc
            .split(',')
            .map(str::trim)
            .filter(|address| !address.is_empty())
            .map(str::to_owned)
            .collect(),
        reply_to: Vec::new(),
        subject: if draft.subject.is_empty() {
            "未命名草稿".into()
        } else {
            draft.subject.clone()
        },
        preview: draft.body_text.chars().take(120).collect(),
        body_text: draft.body_text.clone(),
        received_at: draft.updated_at.clone(),
        unread: false,
        starred: false,
        label_ids: Vec::new(),
        draft: Some(draft),
    })
}

pub fn save(
    conn: &mut Connection,
    draft: &Draft,
    previous_account_id: &str,
    staging: &ComposeStaging,
) -> Result<MessageRow, AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法开始草稿保存"))?;
    let saved = write(&tx, draft, previous_account_id, staging)?;
    let message = project(&tx, saved)?;
    tx.commit()
        .map_err(|_| AppError::storage("草稿保存未完成，请保留当前内容。"))?;
    // 提交成功才回收暂存：内容已落库或附件已从列表移除；失败保留供重试。
    staging.consume_draft(&draft.id);
    Ok(message)
}

pub fn list(conn: &Connection) -> Result<Vec<MessageRow>, AppError> {
    let mut statement = conn.prepare("SELECT id, account_id, recipients_json, subject, body_text, body_html, reply_to_id, updated_at, revision, cc, bcc, attachments_json FROM drafts ORDER BY updated_at DESC, id DESC")
        .map_err(|_| AppError::storage("无法读取本机草稿"))?;
    let drafts = statement
        .query_map([], |row| {
            let recipients: String = row.get(2)?;
            let to: Vec<String> = serde_json::from_str(&recipients).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let attachments: String = row.get(11)?;
            let attachments: Vec<DraftAttachment> =
                serde_json::from_str(&attachments).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        11,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            Ok(Draft {
                id: row.get(0)?,
                account_id: row.get(1)?,
                to: to.join(", "),
                cc: row.get(9)?,
                bcc: row.get(10)?,
                subject: row.get(3)?,
                body_text: row.get(4)?,
                body_html: row.get(5)?,
                reply_to_id: row.get(6)?,
                attachments,
                updated_at: row.get(7)?,
                revision: row.get(8)?,
            })
        })
        .map_err(|_| AppError::storage("无法读取本机草稿"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("本机草稿数据无法读取，请保留数据库后重试。"))?;
    drafts
        .into_iter()
        .map(|draft| project(conn, draft))
        .collect()
}

pub fn import_legacy(
    conn: &mut Connection,
    drafts: &[Draft],
    staging: &ComposeStaging,
) -> Result<(), AppError> {
    if drafts.is_empty() {
        return Ok(());
    }
    if drafts.len() > 100 {
        return Err(AppError::invalid("旧版演示草稿数量无效"));
    }
    // 旧版草稿从不包含附件：拒绝而非静默剥离，避免伪装出没有内容的附件元数据。
    if drafts.iter().any(|draft| !draft.attachments.is_empty()) {
        return Err(AppError::invalid("旧版草稿不包含附件"));
    }
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法导入旧版草稿"))?;
    let completed: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_imports WHERE source_key = ?1)",
            [LEGACY_SOURCE],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法检查草稿导入状态"))?;
    if completed {
        return Ok(());
    }
    for draft in drafts {
        let profile: Option<String> = tx
            .query_row(
                "SELECT profile_json FROM accounts WHERE id = ?1",
                [&draft.account_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| AppError::storage("无法检查旧版草稿账号"))?
            .flatten();
        let is_demo = profile
            .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
            .is_some_and(|p| p["status"] == "demo");
        if !is_demo {
            return Err(AppError::invalid("只能从旧版存储导入演示账号的草稿"));
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM drafts WHERE id = ?1)",
                [&draft.id],
                |row| row.get(0),
            )
            .map_err(|_| AppError::storage("无法检查已有草稿"))?;
        if !exists {
            let mut imported = draft.clone();
            imported.revision = 0;
            let saved = write(&tx, &imported, &imported.account_id, staging)?;
            // Keep the original ordering on import; later edits use the current clock.
            if chrono::DateTime::parse_from_rfc3339(&draft.updated_at).is_ok() {
                tx.execute(
                    "UPDATE drafts SET updated_at = ?1 WHERE id = ?2",
                    params![draft.updated_at, saved.id],
                )
                .map_err(|_| AppError::storage("无法保留草稿时间"))?;
            }
        }
    }
    tx.execute(
        "INSERT INTO local_imports VALUES (?1, ?2)",
        params![LEGACY_SOURCE, crate::now_utc()],
    )
    .map_err(|_| AppError::storage("无法记录草稿导入状态"))?;
    tx.commit().map_err(|_| AppError::storage("草稿导入未完成"))
}

/// 放弃草稿：用户显式点击「放弃草稿」后删除本机草稿行（幂等；已进入发送流程的草稿拒绝删除）。
/// 附件字节随外键级联删除；Rust 暂存区中未落库的内容一并回收。
pub fn delete(conn: &mut Connection, id: &str, staging: &ComposeStaging) -> Result<(), AppError> {
    if !valid_draft_id(id) {
        return Err(AppError::invalid("草稿标识无效"));
    }
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法开始草稿删除"))?;
    let in_outbox: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = ?1)",
            [id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法检查发送状态"))?;
    if in_outbox {
        return Err(AppError::conflict("这封草稿已进入发送流程，不能再放弃。"));
    }
    // 不存在的草稿视为已放弃（未保存过的新草稿没有本机行）。
    tx.execute("DELETE FROM drafts WHERE id = ?1", [id])
        .map_err(|_| AppError::storage("草稿未能从本机删除，请重试。"))?;
    tx.commit()
        .map_err(|_| AppError::storage("草稿删除未完成，请重试。"))?;
    staging.consume_draft(id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        db.execute_batch("INSERT INTO accounts (id, provider, email, display_name, created_at, profile_json) VALUES
          ('personal','gmail','a@example.test','Demo','2026-11-11T00:00:00Z','{\"status\":\"demo\"}'),
          ('real','imap','a@studio.example','Real','2026-11-11T00:00:00Z','{\"status\":\"connected\"}');").unwrap();
        db
    }
    fn draft() -> Draft {
        Draft {
            id: "draft-native".into(),
            account_id: "real".into(),
            to: "friend@example.test".into(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            body_text: "Local only".into(),
            body_html: None,
            reply_to_id: None,
            attachments: vec![],
            updated_at: "2026-11-11T00:00:00Z".into(),
            revision: 0,
        }
    }

    /// 在暂存区写入测试附件并返回其元数据（与选取命令共用同一 stage 入口）。
    fn staged(
        staging: &ComposeStaging,
        draft_id: &str,
        name: &str,
        content: &[u8],
    ) -> DraftAttachment {
        compose_attach::stage(staging, draft_id, name, content.to_vec()).unwrap()
    }

    #[test]
    fn real_account_draft_is_durable_across_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.db");
        let mut db = crate::storage::initialize(&path).unwrap();
        db.execute_batch("INSERT INTO accounts (id,provider,email,display_name,created_at) VALUES ('real','imap','a@studio.example','Real','2026-11-11T00:00:00Z');").unwrap();
        let mut input = draft();
        input.cc = "copy@example.test".into();
        input.bcc = "private@example.test".into();
        let staging = ComposeStaging::default();
        let message = save(&mut db, &input, "real", &staging).unwrap();
        assert_eq!(message.from.email, "a@studio.example");
        assert_eq!(message.subject, "未命名草稿");
        drop(db);
        let db = crate::storage::initialize(&path).unwrap();
        let restored = list(&db).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].draft.as_ref().unwrap().subject, "");
        assert_eq!(restored[0].draft.as_ref().unwrap().revision, 1);
        assert_eq!(restored[0].draft.as_ref().unwrap().cc, "copy@example.test");
        assert_eq!(
            restored[0].draft.as_ref().unwrap().bcc,
            "private@example.test"
        );
        assert!(restored[0].draft.as_ref().unwrap().attachments.is_empty());
    }

    #[test]
    fn stale_writes_and_wrong_account_cannot_overwrite() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let first = save(&mut db, &draft(), "real", &staging)
            .unwrap()
            .draft
            .unwrap();
        assert_eq!(
            save(&mut db, &draft(), "real", &staging).unwrap_err().code,
            "conflict"
        );
        let mut moved = first.clone();
        moved.account_id = "personal".into();
        assert_eq!(
            save(&mut db, &moved, "personal", &staging)
                .unwrap_err()
                .code,
            "conflict"
        );
        let moved = save(&mut db, &moved, "real", &staging)
            .unwrap()
            .draft
            .unwrap();
        assert_eq!(moved.revision, 2);
        assert_eq!(list(&db).unwrap().len(), 1);
        assert_eq!(list(&db).unwrap()[0].account_id, "personal");
        assert_eq!(
            save(&mut db, &first, "real", &staging).unwrap_err().code,
            "conflict"
        );
    }

    #[test]
    fn delete_removes_draft_and_rejects_invalid_ids() {
        let mut db = db();
        let staging = ComposeStaging::default();
        save(&mut db, &draft(), "real", &staging).unwrap();
        delete(&mut db, "draft-native", &staging).unwrap();
        assert!(list(&db).unwrap().is_empty());
        // 幂等：再次删除（草稿已不存在）仍成功。
        delete(&mut db, "draft-native", &staging).unwrap();
        assert_eq!(
            delete(&mut db, "not-a-draft", &staging).unwrap_err().code,
            "invalid"
        );
    }

    #[test]
    fn invalid_account_limits_and_cross_account_reply_are_rejected() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut invalid = draft();
        invalid.account_id = "missing".into();
        assert_eq!(
            save(&mut db, &invalid, "missing", &staging)
                .unwrap_err()
                .code,
            "invalid"
        );
        invalid = draft();
        invalid.body_text = "😀".repeat(25_001);
        assert_eq!(
            save(&mut db, &invalid, "real", &staging).unwrap_err().code,
            "invalid"
        );
        db.execute_batch("INSERT INTO messages (id,account_id,remote_id,thread_id,subject,sender_name,sender_email,received_at) VALUES ('other-message','personal','remote','t','Subject','A','a@example.test','2026-11-11T00:00:00Z');").unwrap();
        invalid = draft();
        invalid.reply_to_id = Some("other-message".into());
        assert_eq!(
            save(&mut db, &invalid, "real", &staging).unwrap_err().code,
            "invalid"
        );
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn legacy_import_is_once_and_never_overwrites_an_existing_draft() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut legacy = draft();
        legacy.account_id = "personal".into();
        import_legacy(&mut db, &[legacy.clone()], &staging).unwrap();
        let mut edited = list(&db).unwrap().remove(0).draft.unwrap();
        edited.body_text = "Newer native content".into();
        save(&mut db, &edited, "personal", &staging).unwrap();
        import_legacy(&mut db, &[legacy.clone()], &staging).unwrap();
        assert_eq!(list(&db).unwrap()[0].body_text, "Newer native content");
        db.execute("DELETE FROM drafts", []).unwrap();
        import_legacy(&mut db, &[legacy], &staging).unwrap();
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn failing_legacy_import_rolls_back_the_entire_batch() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut valid = draft();
        valid.account_id = "personal".into();
        assert!(import_legacy(&mut db, &[valid, draft()], &staging).is_err());
        assert!(list(&db).unwrap().is_empty());
        let receipts: i64 = db
            .query_row("SELECT COUNT(*) FROM local_imports", [], |r| r.get(0))
            .unwrap();
        assert_eq!(receipts, 0);
    }

    #[test]
    fn legacy_import_rejects_attachments_instead_of_silently_dropping() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut legacy = draft();
        legacy.account_id = "personal".into();
        legacy.attachments = vec![DraftAttachment {
            id: "att-x".into(),
            name: "a.txt".into(),
            size: 1,
            media_type: "text/plain".into(),
            sha256: "0".repeat(64),
        }];
        assert_eq!(
            import_legacy(&mut db, &[legacy], &staging)
                .unwrap_err()
                .code,
            "invalid"
        );
    }

    #[test]
    fn v2_migration_preserves_existing_drafts_and_adds_revision() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/001_foundation.sql"))
            .unwrap();
        db.execute_batch(include_str!("../migrations/002_mail_store.sql"))
            .unwrap();
        db.pragma_update(None, "user_version", 2).unwrap();
        db.execute_batch("INSERT INTO accounts (id,provider,email,display_name,created_at) VALUES ('real','imap','a@example.test','A','2026-11-11T00:00:00Z');
          INSERT INTO drafts (id,account_id,subject,body_text,updated_at) VALUES ('draft-existing','real','Keep','Existing body','2026-11-11T00:00:00Z');").unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let restored = list(&db).unwrap().remove(0).draft.unwrap();
        assert_eq!(restored.revision, 1);
        assert_eq!(restored.body_text, "Existing body");
        assert!(restored.cc.is_empty() && restored.bcc.is_empty());
    }

    // ── 撰写附件 ────────────────────────────────────────────────

    fn blob_count(conn: &Connection, draft_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM draft_attachment_blobs WHERE draft_id = ?1",
            [draft_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn attachments_roundtrip_through_save_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.db");
        let mut db = crate::storage::initialize(&path).unwrap();
        db.execute_batch("INSERT INTO accounts (id,provider,email,display_name,created_at) VALUES ('real','imap','a@studio.example','Real','2026-11-11T00:00:00Z');").unwrap();
        let staging = ComposeStaging::default();
        let mut input = draft();
        let content = "附件内容 bytes \u{1F600}".as_bytes().to_vec();
        let attachment = staged(&staging, &input.id, "报告.pdf", &content);
        input.attachments = vec![attachment.clone()];
        let message = save(&mut db, &input, "real", &staging).unwrap();
        assert_eq!(message.draft.as_ref().unwrap().attachments.len(), 1);
        // 暂存已消费；落库内容逐字节一致。
        let stored: Vec<u8> = db
            .query_row(
                "SELECT content FROM draft_attachment_blobs WHERE draft_id = ?1 AND attachment_id = ?2",
                params![input.id, attachment.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, content);
        drop(db);
        // 重开数据库：草稿与附件元数据恢复；暂存随进程内存已清空。
        let mut db = crate::storage::initialize(&path).unwrap();
        let restored = list(&db).unwrap().remove(0).draft.unwrap();
        assert_eq!(restored.attachments, vec![attachment.clone()]);
        assert!(compose_attach::staged_content(
            &ComposeStaging::default(),
            &input.id,
            &attachment.id,
            &attachment.sha256
        )
        .is_none());
        // 后续保存（暂存为空）命中同 sha256 的既有行，成功且不重复写内容。
        let mut edited = restored;
        edited.body_text = "Edited body".into();
        save(&mut db, &edited, "real", &ComposeStaging::default()).unwrap();
        assert_eq!(blob_count(&db, &input.id), 1);
        let stored: Vec<u8> = db
            .query_row(
                "SELECT content FROM draft_attachment_blobs WHERE draft_id = ?1 AND attachment_id = ?2",
                params![input.id, attachment.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, content);
    }

    #[test]
    fn removing_attachment_deletes_blob_and_keeps_referenced_one() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut input = draft();
        let first = staged(&staging, &input.id, "one.txt", b"one");
        let second = staged(&staging, &input.id, "two.txt", b"two");
        input.attachments = vec![first.clone(), second.clone()];
        save(&mut db, &input, "real", &staging).unwrap();
        assert_eq!(blob_count(&db, &input.id), 2);
        // 移除第二个附件后保存（引用已落库的第一个）：子表只剩一行。
        let mut kept = input.clone();
        kept.attachments = vec![first.clone()];
        kept.revision = 1;
        save(&mut db, &kept, "real", &ComposeStaging::default()).unwrap();
        assert_eq!(blob_count(&db, &input.id), 1);
        let remaining: String = db
            .query_row(
                "SELECT attachment_id FROM draft_attachment_blobs WHERE draft_id = ?1",
                [input.id.clone()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, first.id);
        // 删除草稿：附件字节随外键级联清空。
        delete(&mut db, &input.id, &staging).unwrap();
        assert_eq!(blob_count(&db, &input.id), 0);
    }

    #[test]
    fn missing_staged_content_fails_honestly_then_recovers_after_restaging() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut input = draft();
        input.attachments = vec![DraftAttachment {
            id: "att-lost".into(),
            name: "lost.txt".into(),
            size: 4,
            media_type: "text/plain".into(),
            sha256: "a".repeat(64),
        }];
        // 无暂存、无落库：诚实失败，草稿不入库。
        let error = save(&mut db, &input, "real", &staging).unwrap_err();
        assert_eq!(error.code, "invalid");
        assert!(error.message.contains("内容缺失"));
        assert!(list(&db).unwrap().is_empty());
        // 重新暂存后成功；失败不消费暂存。元数据 sha 与暂存内容一致才被接受。
        {
            use sha2::{Digest, Sha256};
            let content = b"data".to_vec();
            input.attachments[0].sha256 = format!("{:x}", Sha256::digest(&content));
            crate::compose_attach::stage_existing(
                &staging,
                &input.id,
                &input.attachments[0],
                content,
            )
            .unwrap();
        }
        save(&mut db, &input, "real", &staging).unwrap();
        assert_eq!(blob_count(&db, &input.id), 1);
    }

    #[test]
    fn invalid_attachment_metadata_and_limits_are_rejected() {
        let mut db = db();
        let staging = ComposeStaging::default();
        let mut input = draft();
        input.attachments = vec![DraftAttachment {
            id: "bad".into(),
            name: "a.txt".into(),
            size: 1,
            media_type: "text/plain".into(),
            sha256: "zz".into(),
        }];
        assert_eq!(
            save(&mut db, &input, "real", &staging).unwrap_err().code,
            "invalid"
        );
        // 合计超限：21 个 1 MiB 附件。
        let mut over = draft();
        over.attachments = (0..21)
            .map(|index| DraftAttachment {
                id: format!("att-{index:04}"),
                name: format!("{index}.bin"),
                size: 1024 * 1024,
                media_type: "application/octet-stream".into(),
                sha256: "a".repeat(64),
            })
            .collect();
        assert_eq!(
            save(&mut db, &over, "real", &staging).unwrap_err().code,
            "invalid"
        );
        // 总量恰好 16 MiB（16 × 1 MiB）合法：校验通过，进入落库阶段因无内容而报缺失。
        let mut full = draft();
        full.attachments = (0..16)
            .map(|index| DraftAttachment {
                id: format!("att-{index:04}"),
                name: format!("{index}.bin"),
                size: 1024 * 1024,
                media_type: "application/octet-stream".into(),
                sha256: "a".repeat(64),
            })
            .collect();
        assert!(save(&mut db, &full, "real", &staging)
            .unwrap_err()
            .message
            .contains("内容缺失"));
        // 重复 id 拒绝。
        full.attachments[1].id = full.attachments[0].id.clone();
        assert_eq!(
            save(&mut db, &full, "real", &staging).unwrap_err().code,
            "invalid"
        );
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn v15_migration_preserves_existing_drafts_and_adds_attachment_columns() {
        let mut db = Connection::open_in_memory().unwrap();
        let scripts: &[&str] = &[
            include_str!("../migrations/001_foundation.sql"),
            include_str!("../migrations/002_mail_store.sql"),
            include_str!("../migrations/003_local_drafts.sql"),
            include_str!("../migrations/004_sync_generation.sql"),
            include_str!("../migrations/005_message_changes.sql"),
            include_str!("../migrations/006_remote_recovery.sql"),
            include_str!("../migrations/007_recovery_scan.sql"),
            include_str!("../migrations/008_fingerprint_provenance.sql"),
            include_str!("../migrations/009_imap_completion.sql"),
            include_str!("../migrations/010_outbox_send.sql"),
            include_str!("../migrations/011_draft_html.sql"),
            include_str!("../migrations/012_draft_recipients.sql"),
            include_str!("../migrations/013_message_reply_headers.sql"),
            include_str!("../migrations/014_imap_round_progress.sql"),
        ];
        for script in scripts {
            db.execute_batch(script).unwrap();
        }
        db.pragma_update(None, "user_version", 14).unwrap();
        db.execute_batch("INSERT INTO accounts (id,provider,email,display_name,created_at) VALUES ('real','imap','a@example.test','A','2026-11-11T00:00:00Z');
          INSERT INTO drafts (id,account_id,subject,body_text,updated_at,revision) VALUES ('draft-existing','real','Keep','Existing body','2026-11-11T00:00:00Z',3);").unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let restored = list(&db).unwrap().remove(0).draft.unwrap();
        assert_eq!(restored.revision, 3);
        assert_eq!(restored.body_text, "Existing body");
        assert!(restored.attachments.is_empty());
        // 迁移不破坏后续保存（含附件）。
        let staging = ComposeStaging::default();
        let mut input = restored;
        input.id = "draft-new".into();
        input.revision = 0;
        input.attachments = vec![staged(&staging, "draft-new", "n.txt", b"v15")];
        save(&mut db, &input, "real", &staging).unwrap();
        assert_eq!(blob_count(&db, "draft-new"), 1);
    }
}
