//! 服务器草稿上传（IMAP APPEND）：本机草稿 → 服务器草稿目录副本（\Draft \Seen）。
//! 独立短会话（不持有 DB 锁）；收件人允许为空（草稿可未完成）；附件随副本完整上传
//! （字节核对 sha256，与发送一致）；上传的是新副本——本机草稿继续独立编辑。
use crate::{credentials, drafts, error::AppError, imap_folders, outbox};
use imap::types::Flag;
use rusqlite::{params, OptionalExtension};

/// 草稿行投影（revision / 主题 / 正文 / 收件人 / 附件元数据 JSON）。
type DraftRow = (
    u32,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
);

#[derive(Debug)]
struct DraftFolder {
    remote_id: String,
    display_name: String,
}

/// 唯一可用的草稿目录：0 个明确报错，多个不猜（与 trash 解析同一保守策略）。
fn resolve_draft_folder(
    conn: &rusqlite::Connection,
    account: &str,
) -> Result<DraftFolder, AppError> {
    let rows: Vec<(String, String)> = conn
        .prepare(
            "SELECT f.remote_id,f.name FROM folders f \
             JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id AND s.available=1 \
             WHERE f.account_id=?1 AND f.kind='drafts'",
        )
        .map_err(|_| AppError::storage("无法读取草稿目录"))?
        .query_map([account], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|_| AppError::storage("无法读取草稿目录"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("无法读取草稿目录"))?;
    match rows.as_slice() {
        [(remote_id, name)] => Ok(DraftFolder {
            remote_id: remote_id.clone(),
            display_name: crate::imap_settings::display_name(name),
        }),
        [] => Err(AppError::invalid(
            "服务器没有可用的草稿目录，无法上传服务器草稿。",
        )),
        _ => Err(AppError::invalid(
            "服务器有多个草稿目录，无法确定上传目标；请核对目录配置。",
        )),
    }
}

/// 上传结果：(字节数, 目标目录显示名)。
pub(crate) fn upload(
    db: &crate::mail_sync::SharedDb,
    account: &str,
    draft_id: &str,
    expected_revision: u32,
) -> Result<(usize, String), AppError> {
    if !drafts::valid_draft_id(draft_id) {
        return Err(AppError::invalid("草稿标识无效"));
    }
    // ── 短锁阶段：草稿、账号、目录解析、附件字节 ───────────────────
    let (revision, subject, body_text, body_html, recipients_json, cc, bcc, attachments_json): DraftRow = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let row: Option<DraftRow> =
            conn.query_row(
                "SELECT revision,subject,body_text,body_html,recipients_json,cc,bcc,attachments_json FROM drafts WHERE id=?1",
                [draft_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| AppError::storage("无法读取草稿"))?;
        row.ok_or_else(|| AppError::invalid("草稿不存在"))
    }?;
    if revision != expected_revision {
        return Err(AppError::conflict("草稿已被修改，请保存后再上传"));
    }
    let (from_name, from_email, provider) = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT display_name,email,provider FROM accounts WHERE id=?1",
                [account],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| AppError::storage("无法读取账号"))?;
        row.ok_or_else(|| AppError::invalid("账号不存在"))
    }?;
    if provider != "imap" {
        return Err(AppError::invalid("服务器草稿上传仅支持 IMAP 账号"));
    }
    let folder = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        resolve_draft_folder(&conn, account)?
    };
    // 附件：字节随副本完整上传；sha256 与元数据核对（与发送同一条完整性规则）。
    let metadata: Vec<drafts::DraftAttachment> = serde_json::from_str(&attachments_json)
        .map_err(|_| AppError::storage("草稿附件数据损坏"))?;
    let mut attachments: Vec<outbox::AttachmentPayload> = Vec::new();
    for item in &metadata {
        let (stored_sha, content): (String, Vec<u8>) = {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            let row: Option<(String, Vec<u8>)> = conn
                .query_row(
                    "SELECT sha256,content FROM draft_attachment_blobs WHERE draft_id=?1 AND attachment_id=?2",
                    params!(draft_id, item.id),
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| AppError::storage("无法读取草稿附件"))?;
            row.ok_or_else(|| {
                AppError::storage(format!("附件「{}」缺失，请重新添加后再上传。", item.name))
            })?
        };
        if stored_sha != item.sha256 || content.len() != item.size {
            return Err(AppError::storage(format!(
                "附件「{}」与本机记录不一致，请重新添加后再上传。",
                item.name
            )));
        }
        attachments.push(outbox::AttachmentPayload {
            name: item.name.clone(),
            media_type: item.media_type.clone(),
            content,
        });
    }
    // 收件人允许为空（草稿可未完成）：与发送不同，不做"至少一位收件人"检查。
    let to: Vec<String> = serde_json::from_str(&recipients_json)
        .map_err(|_| AppError::storage("草稿收件人数据损坏"))?;
    let optional = |raw: &str| -> Result<Vec<String>, AppError> {
        if raw.trim().is_empty() {
            Ok(Vec::new())
        } else {
            outbox::parse_recipients(raw)
        }
    };
    let recipients = outbox::Recipients {
        to,
        cc: optional(&cc)?,
        bcc: optional(&bcc)?,
    };
    let content = outbox::assemble(&outbox::MimeInput {
        from_name: &from_name,
        from_email: &from_email,
        recipients: &recipients,
        subject: &subject,
        body_text: &body_text,
        body_html: body_html.as_deref(),
        // 副本 Message-ID 独立生成：上传是草稿副本而非发送，不复用发送幂等标识。
        message_id: &format!("<{}@auumail.local>", uuid::Uuid::new_v4().simple()),
        reply: None,
        attachments: &attachments,
    });
    // ── 锁外阶段：独立短会话 APPEND（\Draft \Seen） ──────────────────
    let _secret_guard =
        credentials::with_account("imap", account, || credentials::secret_for("imap", account))?;
    let mut session = imap_folders::open_session(db, account)?;
    let result = session.append_with_flags(
        &folder.remote_id,
        content.as_bytes(),
        &[Flag::Draft, Flag::Seen],
    );
    let uploaded = result
        .map(|_| content.len())
        .map_err(|error| crate::imap_limits::network(error, "上传到服务器草稿目录失败"));
    let _ = session.logout();
    uploaded.map(|bytes| (bytes, folder.display_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> crate::mail_sync::SharedDb {
        std::sync::Arc::new(std::sync::Mutex::new({
            let mut conn = rusqlite::Connection::open_in_memory().unwrap();
            crate::storage::migrate(&mut conn).unwrap();
            conn
        }))
    }

    fn seed(db: &crate::mail_sync::SharedDb, provider: &str, folders: &[(&str, &str)]) {
        let conn = db.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id,provider,email,display_name,created_at,profile_json) VALUES              ('acc',?1,'me@example.test','测试账号','2026-11-19T00:00:00Z','{\"status\":\"connected\"}')",
            [provider],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO drafts (id,account_id,recipients_json,subject,body_text,updated_at,revision) VALUES              ('draft-1','acc','[\"friend@example.test\"]','草稿主题','草稿正文','2026-11-19T00:00:00Z',2)",
            [],
        )
        .unwrap();
        for (remote, name) in folders {
            conn.execute(
                "INSERT INTO folders (id,account_id,remote_id,name,kind) VALUES ('acc:f:'||?1,'acc',?1,?2,'drafts')",
                params!(remote, name),
            )
            .unwrap();
            conn.execute(
                "INSERT INTO imap_folder_state (account_id,remote_id,available) VALUES ('acc',?1,1)",
                params!(remote),
            )
            .unwrap();
        }
    }

    #[test]
    fn upload_guards_revision_provider_and_draft_existence() {
        let first = fresh();
        seed(&first, "imap", &[]);
        let error = upload(&first, "acc", "draft-1", 9).unwrap_err();
        assert_eq!(error.code, "conflict");
        assert!(upload(&first, "acc", "draft-missing", 0).is_err());
        assert!(upload(&first, "acc", "draft-1", 2)
            .unwrap_err()
            .message
            .contains("服务器没有可用的草稿目录"));
        let pop3 = fresh();
        seed(&pop3, "pop3", &[]);
        assert!(upload(&pop3, "acc", "draft-1", 2)
            .unwrap_err()
            .message
            .contains("仅支持 IMAP"));
    }

    #[test]
    fn draft_folder_resolution_requires_exactly_one_available_folder() {
        let shared = fresh();
        seed(
            &shared,
            "imap",
            &[("Drafts", "Drafts"), ("草稿", "草稿副本")],
        );
        {
            let conn = shared.lock().unwrap();
            assert!(resolve_draft_folder(&conn, "acc")
                .unwrap_err()
                .message
                .contains("多个草稿目录"));
            conn.execute("DELETE FROM folders WHERE name='草稿副本'", [])
                .unwrap();
            conn.execute(
                "UPDATE imap_folder_state SET available=0 WHERE remote_id='Drafts'",
                [],
            )
            .unwrap();
            assert!(resolve_draft_folder(&conn, "acc")
                .unwrap_err()
                .message
                .contains("没有可用的草稿目录"));
            conn.execute(
                "UPDATE imap_folder_state SET available=1 WHERE remote_id='Drafts'",
                [],
            )
            .unwrap();
        }
        let conn = shared.lock().unwrap();
        let folder = resolve_draft_folder(&conn, "acc").unwrap();
        assert_eq!(folder.remote_id, "Drafts");
        assert_eq!(folder.display_name, "Drafts");
    }
}
