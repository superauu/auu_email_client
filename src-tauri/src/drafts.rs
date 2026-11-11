//! Durable, account-scoped local drafts. Saving never sends or uploads a message.
use crate::{
    error::AppError,
    mail_store::{FromRow, MessageRow},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const LEGACY_SOURCE: &str = "auumail.demo.drafts.v1";
const CONFLICT: &str = "草稿已在其他窗口修改，请保留当前内容并重新打开草稿。";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub to: String,
    pub subject: String,
    pub body_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub revision: u32,
}

fn validate(draft: &Draft) -> Result<(), AppError> {
    if !draft.id.starts_with("draft-")
        || draft.id.len() <= 6
        || draft.id.len() > 100
        || !draft.id[6..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || draft.account_id.is_empty()
        || draft.account_id.len() > 200
        || draft.to.encode_utf16().count() > 1000
        || draft.subject.encode_utf16().count() > 1000
        || draft.body_text.encode_utf16().count() > 50_000
        || draft.revision > 1_000_000_000
        || draft
            .reply_to_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 200)
    {
        return Err(AppError::invalid("草稿字段无效或超出长度限制"));
    }
    Ok(())
}

fn write(conn: &Connection, draft: &Draft, previous_account_id: &str) -> Result<Draft, AppError> {
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
    let recipients = serde_json::to_string(&if saved.to.is_empty() {
        Vec::new()
    } else {
        vec![&saved.to]
    })
    .map_err(|_| AppError::invalid("收件人格式无效"))?;
    conn.execute("INSERT INTO drafts (id, account_id, recipients_json, subject, body_text, reply_to_id, updated_at, revision)
      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
      ON CONFLICT(id) DO UPDATE SET account_id = excluded.account_id, recipients_json = excluded.recipients_json,
      subject = excluded.subject, body_text = excluded.body_text, reply_to_id = excluded.reply_to_id,
      updated_at = excluded.updated_at, revision = excluded.revision",
      params![saved.id, saved.account_id, recipients, saved.subject, saved.body_text, saved.reply_to_id, saved.updated_at, saved.revision])
      .map_err(|_| AppError::storage("草稿未能写入本机，请保留当前内容后重试。"))?;
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
) -> Result<MessageRow, AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法开始草稿保存"))?;
    let saved = write(&tx, draft, previous_account_id)?;
    let message = project(&tx, saved)?;
    tx.commit()
        .map_err(|_| AppError::storage("草稿保存未完成，请保留当前内容。"))?;
    Ok(message)
}

pub fn list(conn: &Connection) -> Result<Vec<MessageRow>, AppError> {
    let mut statement = conn.prepare("SELECT id, account_id, recipients_json, subject, body_text, reply_to_id, updated_at, revision FROM drafts ORDER BY updated_at DESC, id DESC")
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
            Ok(Draft {
                id: row.get(0)?,
                account_id: row.get(1)?,
                to: to.join(", "),
                subject: row.get(3)?,
                body_text: row.get(4)?,
                reply_to_id: row.get(5)?,
                updated_at: row.get(6)?,
                revision: row.get(7)?,
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

pub fn import_legacy(conn: &mut Connection, drafts: &[Draft]) -> Result<(), AppError> {
    if drafts.is_empty() {
        return Ok(());
    }
    if drafts.len() > 100 {
        return Err(AppError::invalid("旧版演示草稿数量无效"));
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
            let saved = write(&tx, &imported, &imported.account_id)?;
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
            subject: String::new(),
            body_text: "Local only".into(),
            reply_to_id: None,
            updated_at: "2026-11-11T00:00:00Z".into(),
            revision: 0,
        }
    }

    #[test]
    fn real_account_draft_is_durable_across_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.db");
        let mut db = crate::storage::initialize(&path).unwrap();
        db.execute_batch("INSERT INTO accounts (id,provider,email,display_name,created_at) VALUES ('real','imap','a@studio.example','Real','2026-11-11T00:00:00Z');").unwrap();
        let message = save(&mut db, &draft(), "real").unwrap();
        assert_eq!(message.from.email, "a@studio.example");
        assert_eq!(message.subject, "未命名草稿");
        drop(db);
        let db = crate::storage::initialize(&path).unwrap();
        let restored = list(&db).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].draft.as_ref().unwrap().subject, "");
        assert_eq!(restored[0].draft.as_ref().unwrap().revision, 1);
    }

    #[test]
    fn stale_writes_and_wrong_account_cannot_overwrite() {
        let mut db = db();
        let first = save(&mut db, &draft(), "real").unwrap().draft.unwrap();
        assert_eq!(
            save(&mut db, &draft(), "real").unwrap_err().code,
            "conflict"
        );
        let mut moved = first.clone();
        moved.account_id = "personal".into();
        assert_eq!(
            save(&mut db, &moved, "personal").unwrap_err().code,
            "conflict"
        );
        let moved = save(&mut db, &moved, "real").unwrap().draft.unwrap();
        assert_eq!(moved.revision, 2);
        assert_eq!(list(&db).unwrap().len(), 1);
        assert_eq!(list(&db).unwrap()[0].account_id, "personal");
        assert_eq!(save(&mut db, &first, "real").unwrap_err().code, "conflict");
    }

    #[test]
    fn invalid_account_limits_and_cross_account_reply_are_rejected() {
        let mut db = db();
        let mut invalid = draft();
        invalid.account_id = "missing".into();
        assert_eq!(
            save(&mut db, &invalid, "missing").unwrap_err().code,
            "invalid"
        );
        invalid = draft();
        invalid.body_text = "😀".repeat(25_001);
        assert_eq!(save(&mut db, &invalid, "real").unwrap_err().code, "invalid");
        db.execute_batch("INSERT INTO messages (id,account_id,remote_id,thread_id,subject,sender_name,sender_email,received_at) VALUES ('other-message','personal','remote','t','Subject','A','a@example.test','2026-11-11T00:00:00Z');").unwrap();
        invalid = draft();
        invalid.reply_to_id = Some("other-message".into());
        assert_eq!(save(&mut db, &invalid, "real").unwrap_err().code, "invalid");
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn legacy_import_is_once_and_never_overwrites_an_existing_draft() {
        let mut db = db();
        let mut legacy = draft();
        legacy.account_id = "personal".into();
        import_legacy(&mut db, &[legacy.clone()]).unwrap();
        let mut edited = list(&db).unwrap().remove(0).draft.unwrap();
        edited.body_text = "Newer native content".into();
        save(&mut db, &edited, "personal").unwrap();
        import_legacy(&mut db, &[legacy.clone()]).unwrap();
        assert_eq!(list(&db).unwrap()[0].body_text, "Newer native content");
        db.execute("DELETE FROM drafts", []).unwrap();
        import_legacy(&mut db, &[legacy]).unwrap();
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn failing_legacy_import_rolls_back_the_entire_batch() {
        let mut db = db();
        let mut valid = draft();
        valid.account_id = "personal".into();
        assert!(import_legacy(&mut db, &[valid, draft()]).is_err());
        assert!(list(&db).unwrap().is_empty());
        let receipts: i64 = db
            .query_row("SELECT COUNT(*) FROM local_imports", [], |r| r.get(0))
            .unwrap();
        assert_eq!(receipts, 0);
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
    }
}
