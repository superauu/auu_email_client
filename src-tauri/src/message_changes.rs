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
    /// 删除到废纸篓（UID MOVE 到唯一 trash 目录；POP3 / 演示仅本机投影）。
    pub delete: Option<bool>,
    /// 移动到指定文件夹（folders.id）。
    pub move_to: Option<String>,
}

/// 移动族字段：彼此互斥（同一条邮件同时只允许一个待处理移动意图），标记字段不受影响。
pub const MOVE_FAMILY: [&str; 3] = ["archive", "delete", "move"];
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
    /// 移动族意图的目标 folders.id（archive 在回写时解析，delete / move 在入队时落库）。
    pub target: Option<String>,
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
    tx.execute("UPDATE message_changes SET state=CASE WHEN field IN ('archive','delete','move') THEN 'unknown' WHEN field='purge' THEN 'unknown' ELSE 'queued' END,error=CASE WHEN field IN ('archive','delete','move','purge') THEN '上次永久删除中断，结果待核对；不会自动再次执行，请核对服务器废纸篓。' ELSE NULL END WHERE state='applying'",[]).map_err(|_|AppError::storage("无法恢复邮件回写"))?;
    tx.commit()
        .map_err(|_| AppError::storage("邮件回写恢复未完成"))
}
pub fn update(
    conn: &mut Connection,
    account: &str,
    id: &str,
    patch: &Patch,
) -> Result<MessageRow, AppError> {
    let move_to = patch
        .move_to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if (patch.unread.is_none()
        && patch.starred.is_none()
        && patch.archive.is_none()
        && patch.delete.is_none()
        && move_to.is_none())
        || patch.archive == Some(false)
        || patch.delete == Some(false)
    {
        return Err(AppError::invalid("请选择有效的邮件状态修改"));
    }
    // 移动族字段一次只允许一个；家族意图彼此互斥。
    let family_count = usize::from(patch.archive.is_some())
        + usize::from(patch.delete.is_some())
        + usize::from(move_to.is_some());
    if family_count > 1 {
        return Err(AppError::invalid("归档、删除与移动一次只能执行一个"));
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
    // 家族互斥：任一移动族意图仍在 applying / unknown 时禁止新移动（防止重复 MOVE）。
    let pending_family: Option<String> = tx
        .query_row(
            "SELECT field FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field IN ('archive','delete','move') AND state IN ('applying','unknown') LIMIT 1",
            params![account, id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法检查移动状态"))?;
    if pending_family.is_some()
        && (patch.archive.is_some() || patch.delete.is_some() || move_to.is_some())
    {
        return Err(AppError::conflict(
            "上一次移动结果待核对，请勿重复移动；可选择仅保留本机。",
        ));
    }
    // delete / move 的目标文件夹在入队时落库；archive 维持回写时解析唯一归档目录的既有约定。
    // 可用性在 imap_folder_state（按 remote_id 关联），不在 folders 表。
    let family_target: Option<String> = if patch.delete.is_some() {
        let mut trash: Vec<String> = tx
            .prepare("SELECT f.id FROM folders f JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id WHERE f.account_id=?1 AND f.kind='trash' AND s.available=1 ORDER BY f.id")
            .map_err(|_| AppError::storage("无法读取废纸篓文件夹"))?
            .query_map(params![account], |row| row.get(0))
            .map_err(|_| AppError::storage("无法读取废纸篓文件夹"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::storage("文件夹数据损坏"))?;
        match trash.len() {
            1 => Some(trash.pop().expect("checked len")),
            0 => {
                return Err(AppError::invalid(
                    "未找到可用的废纸篓文件夹，无法删除到服务器",
                ))
            }
            _ => return Err(AppError::invalid("存在多个废纸篓文件夹，无法确定删除目标")),
        }
    } else if let Some(target) = move_to {
        let (remote_name, available): (String, bool) = tx
            .query_row(
                "SELECT f.remote_id,s.available FROM folders f JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id WHERE f.id=?1 AND f.account_id=?2",
                params![target, account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| AppError::storage("无法读取目标文件夹"))?
            .ok_or_else(|| AppError::invalid("目标文件夹不存在"))?;
        if !available {
            return Err(AppError::invalid("目标文件夹当前不可用"));
        }
        if Some(remote_name.as_str()) == folder.as_deref() {
            return Err(AppError::invalid("邮件已在该文件夹"));
        }
        Some(target.to_string())
    } else {
        None
    };
    if pending_family.is_some()
        && (patch.archive.is_some() || patch.delete.is_some() || move_to.is_some())
    {
        return Err(AppError::conflict(
            "上一次移动结果待核对，请勿重复移动；可选择仅保留本机。",
        ));
    }
    for (field, value, target) in [
        ("unread", patch.unread, None),
        ("starred", patch.starred, None),
        ("archive", patch.archive, None),
        ("delete", patch.delete, family_target.clone()),
        ("move", move_to.map(|_| true), family_target.clone()),
    ] {
        let Some(value) = value else { continue };
        if MOVE_FAMILY.contains(&field) {
            // 新家族意图取代旧的 queued / failed / conflict / local 家族意图；
            // applying / unknown 已被上方守卫拦截，不能覆盖。
            tx.execute(
                "DELETE FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field IN ('archive','delete','move') AND field!=?3 AND state NOT IN ('applying','unknown')",
                params![account, id, field],
            )
            .map_err(|_| AppError::storage("无法清理旧移动意图"))?;
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
        tx.execute("INSERT INTO message_changes (account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,error,target_folder,updated_at)
          VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
          ON CONFLICT(account_id,message_id,field) DO UPDATE SET value=excluded.value,revision=excluded.revision,state=excluded.state,generation=excluded.generation,
          source_folder=excluded.source_folder,source_validity=excluded.source_validity,source_uid=excluded.source_uid,error=excluded.error,target_folder=excluded.target_folder,updated_at=excluded.updated_at",
          params![account,id,field,value,revision+1,state,generation,folder,validity,uid,error,target,crate::now_utc()]).map_err(|_|AppError::storage("邮件修改未能排队"))?;
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
/// 废纸篓永久删除结果：IMAP 真实账号排队回写（UID STORE \Deleted + UID EXPUNGE），
/// POP3 / 演示立即移除本机行（服务器副本不受影响——POP3 仅收信约定）。
#[derive(Serialize, Debug)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum PurgeOutcome {
    Removed,
    Queued(Box<MessageRow>),
}

/// 永久删除入队 / 本机移除。前置：邮件的废纸篓投影成立（物理成员 = 服务器真相；
/// 或本机 delete 意图 = POP3 / 待同步投影），且没有待核对的移动或进行中的永久删除。
/// EXPUNGE 不可逆：applying 中断一律转 unknown，不自动重试。
pub fn purge(conn: &mut Connection, account: &str, id: &str) -> Result<PurgeOutcome, AppError> {
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法保存永久删除"))?;
    let (provider, profile, generation): (String, Option<String>, String) = tx
        .query_row(
            "SELECT provider,profile_json,sync_generation FROM accounts WHERE id=?1",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| AppError::invalid("账号不存在"))?;
    let profile = profile.and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let real = provider == "imap" && profile.as_ref().is_none_or(|p| p["status"] != "demo");
    let (revision, folder, validity, uid, origin, missing): (i64, Option<String>, Option<u32>, Option<u32>, Option<String>, bool) = tx
        .query_row(
            "SELECT state_revision,remote_folder,remote_uid_validity,remote_uid,remote_generation,remote_missing FROM messages WHERE id=?1 AND account_id=?2",
            params![id, account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .map_err(|_| AppError::invalid("邮件不存在或不属于此账号"))?;
    // 废纸篓投影：物理成员（IMAP 服务器真相）或本机 delete 意图（POP3 / 待同步投影）。
    let projected_trash: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM message_folders mf JOIN folders f ON f.account_id=mf.account_id AND f.id=mf.folder_id WHERE mf.account_id=?1 AND mf.message_id=?2 AND f.kind='trash')              OR EXISTS(SELECT 1 FROM message_changes c WHERE c.account_id=?1 AND c.message_id=?2 AND c.field='delete' AND c.value=1)",
            params![account, id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法核对废纸篓投影"))?;
    if !projected_trash {
        return Err(AppError::invalid("邮件不在废纸篓，无法永久删除"));
    }
    // 移动族意图未终结（含 queued / failed：移入废纸篓的 MOVE 尚未在服务器生效）时
    // 不能 EXPUNGE——否则永久删除会作用在原目录而不是废纸篓。POP3 的 local 意图不拦截。
    let pending_move: Option<String> = tx
        .query_row(
            "SELECT field FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field IN ('archive','delete','move') AND state IN ('queued','applying','unknown','failed') LIMIT 1",
            params![account, id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法检查移动状态"))?;
    if pending_move.is_some() {
        return Err(AppError::conflict(
            "移入废纸篓尚未同步完成，请先同步后再永久删除",
        ));
    }
    let pending_purge: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM message_changes WHERE account_id=?1 AND message_id=?2 AND field='purge' AND state IN ('queued','applying','unknown'))",
            params![account, id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法检查永久删除状态"))?;
    if pending_purge {
        return Err(AppError::conflict(
            "永久删除已在进行或结果待核对；不会自动重试",
        ));
    }
    if real {
        let located = !missing
            && folder.is_some()
            && validity.is_some_and(|v| v > 0)
            && uid.is_some_and(|u| u > 0)
            && origin.as_deref() == Some(&generation);
        if !located {
            return Err(AppError::conflict(
                "邮件服务器定位缺失或账号已改变；永久删除需要可信定位，请先重新同步",
            ));
        }
        tx.execute("INSERT INTO message_changes (account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,error,target_folder,updated_at)
          VALUES (?1,?2,'purge',1,?3,'queued',?4,?5,?6,?7,NULL,NULL,?8)
          ON CONFLICT(account_id,message_id,field) DO UPDATE SET value=1,revision=excluded.revision,state='queued',generation=excluded.generation,
          source_folder=excluded.source_folder,source_validity=excluded.source_validity,source_uid=excluded.source_uid,error=NULL,target_folder=NULL,updated_at=excluded.updated_at",
          params![account, id, revision + 1, generation, folder, validity, uid, crate::now_utc()])
          .map_err(|_| AppError::storage("永久删除未能排队"))?;
        tx.execute(
            "UPDATE messages SET state_revision=state_revision+1 WHERE id=?1 AND account_id=?2",
            params![id, account],
        )
        .map_err(|_| AppError::storage("邮件版本未能保存"))?;
        let row = mail_store::message_by_id(&tx, account, id)?;
        tx.commit()
            .map_err(|_| AppError::storage("永久删除提交未完成"))?;
        Ok(PurgeOutcome::Queued(Box::new(row)))
    } else {
        // POP3 / 演示：直接移除本机行。事件经触发器解绑；意图级联清理；
        // POP3 已见 UIDL 集合保留，服务器副本不会被重新下载。
        tx.execute(
            "DELETE FROM messages WHERE id=?1 AND account_id=?2",
            params![id, account],
        )
        .map_err(|_| AppError::storage("本机移除未完成"))?;
        tx.commit()
            .map_err(|_| AppError::storage("本机移除提交未完成"))?;
        Ok(PurgeOutcome::Removed)
    }
}

/// 服务器 EXPUNGE 确认后删除本机行；返回删除行数（0 = 已不存在，同样视为已确认）。
pub fn acknowledge_purge(conn: &Connection, intent: &Intent) -> Result<usize, AppError> {
    conn.execute(
        "DELETE FROM messages WHERE account_id=?1 AND id=?2",
        params![intent.account_id, intent.message_id],
    )
    .map_err(|_| AppError::storage("永久删除本机确认失败"))
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
    let mut stmt=conn.prepare("SELECT account_id,message_id,field,value,revision,generation,source_folder,source_validity,source_uid,target_folder FROM message_changes WHERE account_id=?1 AND state IN ('queued','failed') AND NOT EXISTS(SELECT 1 FROM message_recovery r WHERE r.message_id=message_changes.message_id AND r.state NOT IN ('restored','local')) ORDER BY CASE field WHEN 'archive' THEN 1 WHEN 'delete' THEN 1 WHEN 'move' THEN 1 ELSE 0 END,message_id,field LIMIT 200").map_err(|_|AppError::storage("无法读取待回写修改"))?;
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
                target: r.get(9)?,
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
    fn purge_requires_trash_projection_and_clean_move_state() {
        let mut conn = fixture();
        // 邮件在 INBOX（无废纸篓投影）→ 拒绝。
        let error = purge(&mut conn, "acc", "m").unwrap_err();
        assert_eq!(error.code, "invalid");
        // 物理位于废纸篓目录时允许排队（IMAP 真实路径）。
        conn.execute_batch(
            "INSERT INTO folders VALUES ('tr','acc','Trash','Trash','trash');
          INSERT INTO message_folders VALUES ('acc','m','tr');
          UPDATE messages SET remote_folder='Trash' WHERE id='m';",
        )
        .unwrap();
        match purge(&mut conn, "acc", "m").unwrap() {
            PurgeOutcome::Queued(row) => {
                assert_eq!(row.mailbox, "trash");
                let state: String = conn
                    .query_row(
                        "SELECT state FROM message_changes WHERE message_id='m' AND field='purge'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(state, "queued");
            }
            PurgeOutcome::Removed => panic!("IMAP 账号应排队而非立即移除"),
        }
        // 已有 purge 排队时再次 purge → conflict（不重复 EXPUNGE）。
        let error = purge(&mut conn, "acc", "m").unwrap_err();
        assert_eq!(error.code, "conflict");
        // unknown 后不自动重试：显式清理前一直 conflict。
        conn.execute(
            "UPDATE message_changes SET state='unknown' WHERE message_id='m' AND field='purge'",
            [],
        )
        .unwrap();
        let error = purge(&mut conn, "acc", "m").unwrap_err();
        assert_eq!(error.code, "conflict");
    }

    #[test]
    fn purge_blocked_until_pending_delete_move_syncs_and_removes_row_locally_for_pop3() {
        let mut conn = fixture();
        // queued delete 意图（服务器还没移到废纸篓）→ purge 必须先同步。
        conn.execute_batch("INSERT INTO folders VALUES ('tr','acc','Trash','Trash','trash');
          INSERT INTO message_folders VALUES ('acc','m','tr');
          UPDATE messages SET remote_folder='INBOX' WHERE id='m';
          INSERT INTO message_changes (account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,target_folder,error,updated_at)
          VALUES ('acc','m','delete',1,1,'queued','gen','INBOX',10,1,'tr',NULL,'2026-11-11T00:00:00Z');")
            .unwrap();
        let error = purge(&mut conn, "acc", "m").unwrap_err();
        assert_eq!(error.code, "conflict");
        // POP3 路径：delete 意图为 local（本机投影），purge 直接移除本机行。
        let mut pop3 = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut pop3).unwrap();
        pop3.execute_batch("INSERT INTO accounts(id,provider,email,display_name,profile_json,created_at) VALUES ('p','pop3','p@example.test','P','{\"status\":\"offline\"}','2026-11-11T00:00:00Z');
          INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,body_text,received_at,unread,starred,remote_folder,remote_uid_validity,remote_uid,remote_generation)
          SELECT 'pm','p','pop3:1','pm','Test','S','s@example.test','B','2026-11-11T00:00:00Z',1,0,'INBOX',0,1,sync_generation FROM accounts WHERE id='p';
          INSERT INTO message_changes (account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,target_folder,error,updated_at)
          VALUES ('p','pm','delete',1,1,'local','gen','INBOX',0,1,NULL,NULL,'2026-11-11T00:00:00Z');")
            .unwrap();
        match purge(&mut pop3, "p", "pm").unwrap() {
            PurgeOutcome::Removed => {
                let exists: bool = pop3
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM messages WHERE id='pm')",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert!(!exists, "POP3 永久删除应移除本机行");
                let intents: i64 = pop3
                    .query_row(
                        "SELECT COUNT(*) FROM message_changes WHERE message_id='pm'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(intents, 0, "意图应级联清理");
            }
            PurgeOutcome::Queued(_) => panic!("POP3 应本机移除而非排队"),
        }
    }

    #[test]
    fn acknowledge_purge_removes_message_row_and_cascades_intents() {
        let mut conn = fixture();
        conn.execute_batch(
            "INSERT INTO folders VALUES ('tr','acc','Trash','Trash','trash');
          INSERT INTO message_folders VALUES ('acc','m','tr');
          UPDATE messages SET remote_folder='Trash' WHERE id='m';",
        )
        .unwrap();
        let intent = purge(&mut conn, "acc", "m").unwrap();
        let PurgeOutcome::Queued(_) = intent else {
            panic!("IMAP 应排队");
        };
        let intents = Intent {
            account_id: "acc".into(),
            message_id: "m".into(),
            field: "purge".into(),
            value: true,
            revision: 2,
            generation: "gen".into(),
            folder: "Trash".into(),
            validity: 10,
            uid: 1,
            target: None,
        };
        let removed = acknowledge_purge(&conn, &intents).unwrap();
        assert_eq!(removed, 1);
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id='m')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!exists);
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_changes WHERE message_id='m'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "意图应级联清理");
    }

    #[test]
    fn recover_marks_interrupted_purge_unknown() {
        let mut conn = fixture();
        conn.execute_batch(
            "INSERT INTO folders VALUES ('tr','acc','Trash','Trash','trash');
          INSERT INTO message_folders VALUES ('acc','m','tr');
          UPDATE messages SET remote_folder='Trash' WHERE id='m';",
        )
        .unwrap();
        purge(&mut conn, "acc", "m").unwrap();
        conn.execute(
            "UPDATE message_changes SET state='applying' WHERE message_id='m' AND field='purge'",
            [],
        )
        .unwrap();
        recover(&mut conn).unwrap();
        let (state, error): (String, Option<String>) = conn
            .query_row(
                "SELECT state,error FROM message_changes WHERE message_id='m' AND field='purge'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            state, "unknown",
            "中断的 EXPUNGE 必须转 unknown，不自动重试"
        );
        assert!(error.is_some());
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
                    delete: None,
                    move_to: None,
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

    fn add_folder(conn: &Connection, id: &str, remote: &str, name: &str, kind: &str) {
        conn.execute(
            "INSERT INTO folders (id,account_id,remote_id,name,kind) VALUES (?1,'acc',?2,?3,?4)",
            params![id, remote, name, kind],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO imap_folder_state (account_id,remote_id,available) VALUES ('acc',?1,1)",
            params![remote],
        )
        .unwrap();
    }

    #[test]
    fn delete_requires_one_available_trash_folder_and_persists_target() {
        let mut conn = fixture();
        // 无废纸篓：拒绝删除，不产生意图。
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                delete: Some(true),
                ..Default::default()
            }
        )
        .is_err());
        add_folder(&conn, "acc:folder:Trash", "Trash", "Trash", "trash");
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                delete: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.mailbox, "trash");
        let intent = &pending(&conn, "acc").unwrap()[0];
        assert_eq!(intent.field, "delete");
        assert_eq!(intent.target.as_deref(), Some("acc:folder:Trash"));
        // 两个废纸篓：拒绝并说明，不猜目标。
        add_folder(&conn, "acc:folder:Bin", "Bin", "Bin", "trash");
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                unread: Some(false),
                ..Default::default()
            }
        )
        .is_ok());
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                delete: Some(true),
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn move_validates_target_and_supersedes_previous_family_intent() {
        let mut conn = fixture();
        add_folder(&conn, "acc:folder:Work", "Work", "Work", "other");
        // 目标不存在 / 与源相同：拒绝。
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                move_to: Some("missing".into()),
                ..Default::default()
            }
        )
        .is_err());
        let inbox: String = conn
            .query_row("SELECT id FROM folders WHERE kind='inbox'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                move_to: Some(inbox.clone()),
                ..Default::default()
            }
        )
        .is_err());
        // 合法移动：目标落库， mailbox 投影为目标 kind。
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                move_to: Some("acc:folder:Work".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.mailbox, "other");
        // 旧的 queued 移动意图被新归档意图取代（同族互斥，非 applying/unknown 可替换）。
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                archive: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.mailbox, "archive");
        let intents = pending(&conn, "acc").unwrap();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].field, "archive");
        assert!(intents[0].target.is_none());
    }

    #[test]
    fn applying_or_unknown_family_intent_blocks_new_moves_but_not_flags() {
        let mut conn = fixture();
        add_folder(&conn, "acc:folder:Trash", "Trash", "Trash", "trash");
        update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                delete: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute("UPDATE message_changes SET state='unknown'", [])
            .unwrap();
        // 家族意图 unknown：新移动被拒，标记仍可修改。
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
        assert!(update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                move_to: Some("acc:folder:Trash".into()),
                ..Default::default()
            }
        )
        .is_err());
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
        assert!(saved.starred);
        // 恢复为 queued 后，新家族意图可替换旧的 queued 意图。
        conn.execute(
            "UPDATE message_changes SET state='queued' WHERE field='delete'",
            [],
        )
        .unwrap();
        let saved = update(
            &mut conn,
            "acc",
            "m",
            &Patch {
                archive: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.mailbox, "archive");
        // 标记意图排序在前，取家族意图断言。
        let family = pending(&conn, "acc")
            .unwrap()
            .into_iter()
            .find(|intent| MOVE_FAMILY.contains(&intent.field.as_str()))
            .expect("family intent");
        assert_eq!(family.field, "archive");
        assert!(family.target.is_none());
    }
}
