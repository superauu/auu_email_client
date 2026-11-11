//! Durable local intent; remote I/O lives in imap_writeback, never under these transactions.
use crate::{
    error::AppError,
    mail_store::{self, MessageRow},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    pub unread: Option<bool>,
    pub starred: Option<bool>,
    pub archive: Option<bool>,
}
#[derive(Debug, Serialize)]
pub struct ChangeStatus {
    pub field: String,
    pub state: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Intent {
    pub account_id: String,
    pub message_id: String,
    pub field: String,
    pub value: bool,
    pub revision: i64,
    pub generation: String,
    pub folder: String,
    pub validity: u32,
    pub uid: u32,
}

pub fn statuses(conn: &Connection, id: &str) -> Result<Vec<ChangeStatus>, AppError> {
    let mut stmt = conn
        .prepare("SELECT field,state,error FROM message_changes WHERE message_id=?1 ORDER BY field")
        .map_err(|_| AppError::storage("无法读取邮件回写状态"))?;
    let result = stmt
        .query_map([id], |row| {
            Ok(ChangeStatus {
                field: row.get(0)?,
                state: row.get(1)?,
                error: row.get(2)?,
            })
        })
        .map_err(|_| AppError::storage("无法读取邮件回写状态"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("邮件回写状态损坏"));
    result
}
pub fn recover(conn: &mut Connection) -> Result<(), AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法恢复邮件回写"))?;
    tx.execute("UPDATE messages SET state_revision=state_revision+1 WHERE id IN (SELECT message_id FROM message_changes WHERE state='applying')",[]).map_err(|_|AppError::storage("无法恢复邮件回写"))?;
    tx.execute("UPDATE message_changes SET state=CASE WHEN field='archive' THEN 'unknown' ELSE 'queued' END,error=CASE WHEN field='archive' THEN '上次归档中断，结果待核对；不会自动再次移动。' ELSE NULL END WHERE state='applying'",[]).map_err(|_|AppError::storage("无法恢复邮件回写"))?;
    tx.commit()
        .map_err(|_| AppError::storage("邮件回写恢复未完成"))
}
pub fn update(
    conn: &mut Connection,
    account: &str,
    id: &str,
    patch: &Patch,
) -> Result<MessageRow, AppError> {
    if patch.unread.is_none() && patch.starred.is_none() && patch.archive.is_none()
        || patch.archive == Some(false)
    {
        return Err(AppError::invalid("请选择有效的邮件状态修改"));
    }
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法保存邮件状态"))?;
    let (provider, profile, generation): (String, Option<String>, String) = tx
        .query_row(
            "SELECT provider,profile_json,sync_generation FROM accounts WHERE id=?1",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| AppError::invalid("账号不存在"))?;
    let (revision,folder,validity,uid,origin):(i64,Option<String>,Option<u32>,Option<u32>,Option<String>)=tx.query_row("SELECT state_revision,remote_folder,remote_uid_validity,remote_uid,remote_generation FROM messages WHERE id=?1 AND account_id=?2",params![id,account],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).map_err(|_|AppError::invalid("邮件不存在或不属于此账号"))?;
    let profile = profile.and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let real = provider == "imap" && profile.as_ref().is_none_or(|p| p["status"] != "demo");
    let missing: bool = tx
        .query_row(
            "SELECT remote_missing FROM messages WHERE account_id=?1 AND id=?2",
            params![account, id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::storage("无法读取远端存在状态"))?;
    let located = !missing
        && folder.is_some()
        && validity.is_some_and(|uid| uid > 0)
        && uid.is_some_and(|uid| uid > 0)
        && origin.as_deref() == Some(&generation);
    for (field, value) in [
        ("unread", patch.unread),
        ("starred", patch.starred),
        ("archive", patch.archive),
    ] {
        let Some(value) = value else { continue };
        if field == "archive" {
            let prior:Option<String>=tx.query_row("SELECT state FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field='archive'",params![account,id],|r|r.get(0)).optional().map_err(|_|AppError::storage("无法检查归档状态"))?;
            if prior
                .as_deref()
                .is_some_and(|s| matches!(s, "applying" | "unknown"))
            {
                return Err(AppError::conflict(
                    "归档结果待核对，请勿重复移动；可选择仅保留本机。",
                ));
            }
        } else {
            let sql = if field == "unread" {
                "UPDATE messages SET unread=?1 WHERE id=?2 AND account_id=?3"
            } else {
                "UPDATE messages SET starred=?1 WHERE id=?2 AND account_id=?3"
            };
            tx.execute(sql, params![value, id, account])
                .map_err(|_| AppError::storage("邮件状态未能保存"))?;
        }
        let state = if !real {
            "local"
        } else if located {
            "queued"
        } else {
            "conflict"
        };
        let error = if real && !located {
            Some("邮件服务器定位缺失或账号已改变；已保留本机修改，请重新同步后核对。")
        } else {
            None
        };
        tx.execute("INSERT INTO message_changes (account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,error,updated_at)
          VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
          ON CONFLICT(account_id,message_id,field) DO UPDATE SET value=excluded.value,revision=excluded.revision,state=excluded.state,generation=excluded.generation,
          source_folder=excluded.source_folder,source_validity=excluded.source_validity,source_uid=excluded.source_uid,error=excluded.error,target_folder=NULL,updated_at=excluded.updated_at",
          params![account,id,field,value,revision+1,state,generation,folder,validity,uid,error,crate::now_utc()]).map_err(|_|AppError::storage("邮件修改未能排队"))?;
    }
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1 AND account_id=?2",
        params![id, account],
    )
    .map_err(|_| AppError::storage("邮件版本未能保存"))?;
    let result = mail_store::message_by_id(&tx, account, id)?;
    tx.commit()
        .map_err(|_| AppError::storage("邮件状态提交未完成"))?;
    Ok(result)
}
pub fn keep_local(conn: &mut Connection, account: &str, id: &str) -> Result<MessageRow, AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法保存本机状态"))?;
    mail_store::message_by_id(&tx, account, id)?;
    let applying:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM message_changes WHERE account_id=?1 AND message_id=?2 AND state='applying')",params![account,id],|r|r.get(0)).map_err(|_|AppError::storage("无法检查回写状态"))?;
    if applying {
        return Err(AppError::conflict("回写正在进行，请等待结果后处理。"));
    }
    tx.execute("UPDATE message_changes SET state='local',error=NULL,revision=revision+1 WHERE account_id=?1 AND message_id=?2",params![account,id]).map_err(|_|AppError::storage("无法保留本机修改"))?;
    tx.execute("UPDATE message_recovery SET state='local',proof_json=NULL,detail='已保留本机，停止远端定位恢复。' WHERE account_id=?1 AND message_id=?2",params![account,id]).map_err(|_|AppError::storage("无法取消定位核对"))?;
    tx.execute(
        "DELETE FROM recovery_scan WHERE account_id=?1 AND message_id=?2",
        params![account, id],
    )
    .map_err(|_| AppError::storage("无法取消分页核对"))?;
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE account_id=?1 AND id=?2",
        params![account, id],
    )
    .map_err(|_| AppError::storage("无法保存邮件版本"))?;
    let result = mail_store::message_by_id(&tx, account, id)?;
    tx.commit()
        .map_err(|_| AppError::storage("本机修改提交失败"))?;
    Ok(result)
}

pub fn pending(conn: &Connection, account: &str) -> Result<Vec<Intent>, AppError> {
    let mut stmt=conn.prepare("SELECT account_id,message_id,field,value,revision,generation,source_folder,source_validity,source_uid FROM message_changes WHERE account_id=?1 AND state IN ('queued','failed') AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=message_changes.message_id AND r.state NOT IN ('restored','local')) ORDER BY CASE field WHEN 'archive' THEN 1 ELSE 0 END,message_id,field LIMIT 200").map_err(|_|AppError::storage("无法读取待回写修改"))?;
    let result = stmt
        .query_map([account], |r| {
            Ok(Intent {
                account_id: r.get(0)?,
                message_id: r.get(1)?,
                field: r.get(2)?,
                value: r.get(3)?,
                revision: r.get(4)?,
                generation: r.get(5)?,
                folder: r.get(6)?,
                validity: r.get(7)?,
                uid: r.get(8)?,
            })
        })
        .map_err(|_| AppError::storage("无法读取待回写修改"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("回写定位数据无效"));
    result
}
pub fn current(conn: &Connection, intent: &Intent) -> Result<bool, AppError> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM message_changes c JOIN accounts a ON a.id=c.account_id WHERE c.account_id=?1 AND c.message_id=?2 AND c.field=?3 AND c.revision=?4 AND c.generation=?5 AND a.sync_generation=?5 AND c.state IN ('queued','failed','applying') AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=c.message_id AND r.state NOT IN ('restored','local')))",
      params![intent.account_id,intent.message_id,intent.field,intent.revision,intent.generation],|r|r.get(0)).map_err(|_|AppError::storage("无法核对待回写修改"))
}
pub fn transition(
    conn: &mut Connection,
    intent: &Intent,
    state: &str,
    error: Option<&str>,
    target: Option<&str>,
) -> Result<bool, AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法更新回写状态"))?;
    let changed=tx.execute("UPDATE message_changes SET state=?1,error=?2,target_folder=COALESCE(?3,target_folder),updated_at=?4 WHERE account_id=?5 AND message_id=?6 AND field=?7 AND revision=?8 AND generation=?9",
        params![state,error,target,crate::now_utc(),intent.account_id,intent.message_id,intent.field,intent.revision,intent.generation]).map_err(|_|AppError::storage("无法更新回写状态"))?;
    if changed > 0 {
        tx.execute(
            "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1 AND account_id=?2",
            params![intent.message_id, intent.account_id],
        )
        .map_err(|_| AppError::storage("无法更新邮件版本"))?;
    }
    tx.commit()
        .map_err(|_| AppError::storage("回写状态未能提交"))?;
    Ok(changed > 0)
}
pub fn acknowledge_flag(conn: &mut Connection, intent: &Intent) -> Result<(), AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法确认邮件标记"))?;
    let generation: Option<String> = tx
        .query_row(
            "SELECT sync_generation FROM accounts WHERE id=?1",
            [&intent.account_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法核对账号"))?;
    if generation.as_deref() != Some(&intent.generation) {
        return Err(AppError::conflict("账号设置已改变，旧回写结果未应用。"));
    }
    tx.execute("DELETE FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field=?3 AND revision=?4",params![intent.account_id,intent.message_id,intent.field,intent.revision]).map_err(|_|AppError::storage("无法确认标记回写"))?;
    tx.execute(
        "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1 AND account_id=?2",
        params![intent.message_id, intent.account_id],
    )
    .map_err(|_| AppError::storage("无法保存标记版本"))?;
    tx.commit().map_err(|_| AppError::storage("标记确认未完成"))
}
pub fn observe_flags(
    conn: &Connection,
    id: &str,
    expected_revision: i64,
    unread: bool,
    starred: bool,
) -> Result<(), AppError> {
    conn.execute("UPDATE messages SET unread=COALESCE((SELECT value FROM message_changes WHERE message_id=?1 AND field='unread'),?2),starred=COALESCE((SELECT value FROM message_changes WHERE message_id=?1 AND field='starred'),?3),state_revision=state_revision+1 WHERE id=?1 AND state_revision=?4 AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=?1 AND r.state!='restored')",
      params![id,unread,starred,expected_revision]).map_err(|_|AppError::storage("无法保存服务器标记"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn fixture() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        conn
    }
    pub(crate) fn seed(conn: &Connection) {
        conn.execute_batch("INSERT INTO accounts(id,provider,email,display_name,profile_json,created_at) VALUES ('acc','imap','test@example.test','Test','{\"status\":\"offline\"}','2026-11-11T00:00:00Z');
          INSERT INTO folders VALUES ('in','acc','INBOX','INBOX','inbox'),('acc:folder:Archive','acc','Archive','Archive','archive');
          INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,body_text,received_at,unread,starred,remote_folder,remote_uid_validity,remote_uid,remote_generation)
          SELECT 'm','acc','imap:inbox:10:1','m','Test','Sender','sender@example.test','Body','2026-11-11T00:00:00Z',1,0,'INBOX',10,1,sync_generation FROM accounts WHERE id='acc';
          INSERT INTO message_folders VALUES ('acc','m','in');").unwrap();
        let has_provenance:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='locator_verified')",[],|r|r.get(0)).unwrap();
        if has_provenance {
            conn.execute("UPDATE messages SET locator_verified=1", [])
                .unwrap();
        }
    }
    #[test]
    fn offline_changes_survive_reopen_and_are_isolated_by_account() {
        let path =
            std::env::temp_dir().join(format!("auumail-changes-{}.sqlite", uuid::Uuid::new_v4()));
        {
            let mut conn = crate::storage::initialize(&path).unwrap();
            seed(&conn);
            let saved = update(
                &mut conn,
                "acc",
                "m",
                &Patch {
                    unread: Some(false),
                    starred: Some(true),
                    archive: Some(true),
                },
            )
            .unwrap();
            assert!(!saved.unread && saved.starred);
            assert_eq!(saved.mailbox, "archive");
            assert_eq!(saved.changes.len(), 3);
            assert_eq!(saved.changes[0].state, "queued");
            assert!(update(
                &mut conn,
                "another",
                "m",
                &Patch {
                    starred: Some(false),
                    ..Default::default()
                }
            )
            .is_err());
        }
        let conn = crate::storage::initialize(&path).unwrap();
        let saved = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!saved.unread && saved.starred);
        assert_eq!(saved.mailbox, "archive");
        assert_eq!(pending(&conn, "acc").unwrap().len(), 3);
        for (kind, count) in [("inbox", 0), ("archive", 1)] {
            assert_eq!(
                mail_store::page_messages(
                    &conn,
                    &mail_store::MessageQuery {
                        mailbox: Some(kind.into()),
                        ..Default::default()
                    }
                )
                .unwrap()
                .items
                .len(),
                count
            );
        }
        drop(conn);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn old_ack_and_server_snapshot_cannot_clear_or_overwrite_newer_edit() {
        let mut conn = fixture();
        update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let old = pending(&conn, "acc").unwrap().remove(0);
        let snapshot = mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .state_revision;
        update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        acknowledge_flag(&mut conn, &old).unwrap();
        observe_flags(&conn, "m", snapshot, false, true).unwrap();
        let saved = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!saved.starred);
        assert!(saved.unread);
        assert_eq!(saved.changes.len(), 1);
        observe_flags(&conn, "m", saved.state_revision, false, true).unwrap();
        let saved = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!saved.unread && !saved.starred);
    }
    #[test]
    fn crash_recovery_retries_idempotent_flags_but_never_repeats_move() {
        let mut conn = fixture();
        update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                unread: Some(false),
                archive: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        for intent in pending(&conn, "acc").unwrap() {
            transition(&mut conn, &intent, "applying", None, None).unwrap();
        }
        assert!(keep_local(&mut conn, "acc", "m").is_err());
        recover(&mut conn).unwrap();
        let states = statuses(&conn, "m").unwrap();
        assert_eq!(states[0].state, "unknown");
        assert_eq!(states[1].state, "queued");
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                archive: Some(true),
                ..Default::default()
            }
        )
        .is_err());
        let saved = keep_local(&mut conn, "acc", "m").unwrap();
        assert!(saved.changes.iter().all(|change| change.state == "local"));
        assert!(pending(&conn, "acc").unwrap().is_empty());
    }
    #[test]
    fn missing_locator_and_changed_account_are_conflicts_not_guessed_uids() {
        let mut conn = fixture();
        conn.execute("UPDATE messages SET remote_uid=NULL", [])
            .unwrap();
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.changes[0].state, "conflict");
        assert!(pending(&conn, "acc").unwrap().is_empty());
        conn.execute("UPDATE messages SET remote_uid=1", [])
            .unwrap();
        conn.execute("UPDATE accounts SET email='changed@example.test'", [])
            .unwrap();
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                unread: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(saved.changes.iter().all(|c| c.state == "conflict"));
    }
    #[test]
    fn old_transition_cannot_touch_recreated_account_with_same_message_revision() {
        let mut conn = fixture();
        update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let old = pending(&conn, "acc").unwrap().remove(0);
        conn.execute("DELETE FROM accounts WHERE id='acc'", [])
            .unwrap();
        seed(&conn);
        update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!transition(&mut conn, &old, "conflict", Some("old response"), None).unwrap());
        assert_eq!(statuses(&conn, "m").unwrap()[0].state, "queued");
    }

    #[test]
    fn v4_upgrade_preserves_cached_content_and_refuses_to_guess_write_locator() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in [
            include_str!("../migrations/001_foundation.sql"),
            include_str!("../migrations/002_mail_store.sql"),
            include_str!("../migrations/003_local_drafts.sql"),
            include_str!("../migrations/004_sync_generation.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4).unwrap();
        conn.execute_batch("INSERT INTO accounts(id,provider,email,display_name,profile_json,created_at) VALUES ('acc','imap','test@example.test','Test','{\"status\":\"offline\"}','2026-11-11T00:00:00Z');
          INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,body_text,received_at) VALUES ('m','acc','imap:inbox:10:1','m','Legacy','Sender','sender@example.test','Preserved body','2026-11-11T00:00:00Z');").unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.body_text, "Preserved body");
        assert!(saved.starred);
        assert_eq!(saved.changes[0].state, "conflict");
        assert!(pending(&conn, "acc").unwrap().is_empty());
    }

    #[test]
    fn intent_insert_failure_rolls_back_local_flags_and_revision() {
        let mut conn = fixture();
        conn.execute_batch("CREATE TRIGGER fail_change BEFORE INSERT ON message_changes BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                starred: Some(true),
                ..Default::default()
            }
        )
        .is_err());
        let saved = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!saved.starred);
        assert_eq!(saved.state_revision, 0);
        assert!(saved.changes.is_empty());
    }
}
