//! 用户自建标签（labels 表中 scope='local' 且无账号归属的全局标签）：纯本机注解，
//! 同步从不读写、无 IMAP 回写。打在缓存邮件行上，邮件行被同步清理（UIDVALIDITY
//! 变更等）时随级联删除——这是有意的取舍，不伪造服务器标签语义。
use crate::error::AppError;
use crate::mail_store::{message_by_id, LabelRow, MessageRow};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

/// 可管理标签的守卫条件：全局 + 本地 scope（演示种子的旧标签同样满足，但前端按
/// id 前缀区分是否展示，见 realWorkspace）。
const MANAGEABLE_GUARD: &str = "scope='local' AND account_id IS NULL";
/// 用户新建标签 id 前缀：realWorkspace 据此始终展示用户自建标签（历史演示标签
/// 仅在真实邮件使用时展示，见 docs/STATUS 隔离批次）。
pub const USER_LABEL_PREFIX: &str = "lbl:";
const MAX_NAME_CHARS: usize = 64;
const LABEL_COLORS: [&str; 3] = ["teal", "blue", "amber"];

fn validate_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("标签名称不能为空"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::invalid("标签名称过长"));
    }
    Ok(name.to_string())
}

fn validate_color(color: &str) -> Result<(), AppError> {
    if LABEL_COLORS.contains(&color) {
        Ok(())
    } else {
        Err(AppError::invalid("不支持的标签颜色"))
    }
}

fn label_row(conn: &Connection, id: &str) -> Result<LabelRow, AppError> {
    conn.query_row(
        &format!("SELECT id, account_id, remote_id, name, color, scope FROM labels WHERE id=?1 AND {MANAGEABLE_GUARD}"),
        params![id],
        |row| {
            Ok(LabelRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                remote_id: row.get(2)?,
                name: row.get(3)?,
                color: row.get(4)?,
                scope: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|_| AppError::storage("无法读取标签"))?
    .ok_or_else(|| AppError::invalid("标签不存在"))
}

/// 名称在可管理标签集合内唯一（trim 后精确比较）；exclude 用于重命名时排除自身。
fn ensure_unique_name(
    conn: &Connection,
    name: &str,
    exclude: Option<&str>,
) -> Result<(), AppError> {
    let exists: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM labels WHERE name=?1 AND id<>COALESCE(?2,'') AND {MANAGEABLE_GUARD})"
            ),
            params![name, exclude],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法核对标签名称"))?;
    if exists {
        Err(AppError::conflict("已存在同名标签"))
    } else {
        Ok(())
    }
}

pub fn create(conn: &Connection, name: &str, color: &str) -> Result<LabelRow, AppError> {
    let name = validate_name(name)?;
    validate_color(color)?;
    ensure_unique_name(conn, &name, None)?;
    let id = format!("{USER_LABEL_PREFIX}{}", Uuid::new_v4());
    conn.execute(
        "INSERT INTO labels(id, account_id, remote_id, name, color, scope) VALUES (?1, NULL, NULL, ?2, ?3, 'local')",
        params![id, name, color],
    )
    .map_err(|_| AppError::storage("无法保存标签"))?;
    label_row(conn, &id)
}

pub fn rename(
    conn: &Connection,
    id: &str,
    name: &str,
    color: Option<&str>,
) -> Result<LabelRow, AppError> {
    let name = validate_name(name)?;
    if let Some(color) = color {
        validate_color(color)?;
    }
    label_row(conn, id)?;
    ensure_unique_name(conn, &name, Some(id))?;
    if let Some(color) = color {
        conn.execute(
            "UPDATE labels SET name=?2, color=?3 WHERE id=?1",
            params![id, name, color],
        )
        .map_err(|_| AppError::storage("无法保存标签"))?;
    } else {
        conn.execute("UPDATE labels SET name=?2 WHERE id=?1", params![id, name])
            .map_err(|_| AppError::storage("无法保存标签"))?;
    }
    label_row(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), AppError> {
    label_row(conn, id)?;
    conn.execute("DELETE FROM labels WHERE id=?1", params![id])
        .map_err(|_| AppError::storage("无法删除标签"))?;
    Ok(())
}

/// 整体替换一封邮件的标签集合（前端传全量集合；空集合 = 清除全部标签）。
/// 校验在写入前完成：邮件必须存在且归属该账号；每个标签必须是可管理标签。
pub fn set_for_message(
    conn: &mut Connection,
    account_id: &str,
    message_id: &str,
    label_ids: &[String],
) -> Result<MessageRow, AppError> {
    // 去重并校验全部标签（任何未知 / 不可管理标签都拒绝，不部分提交）。
    let mut seen = std::collections::HashSet::new();
    let mut resolved: Vec<&str> = Vec::new();
    for id in label_ids {
        if !seen.insert(id.as_str()) {
            continue;
        }
        label_row(conn, id)?;
        resolved.push(id.as_str());
    }
    let message_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND account_id=?2)",
            params![message_id, account_id],
            |row| row.get(0),
        )
        .map_err(|_| AppError::storage("无法读取邮件"))?;
    if !message_exists {
        return Err(AppError::invalid("邮件不存在"));
    }
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法保存标签"))?;
    tx.execute(
        "DELETE FROM message_labels WHERE message_id=?1",
        params![message_id],
    )
    .map_err(|_| AppError::storage("无法保存标签"))?;
    for id in &resolved {
        tx.execute(
            "INSERT INTO message_labels(message_id, label_id) VALUES (?1, ?2)",
            params![message_id, id],
        )
        .map_err(|_| AppError::storage("无法保存标签"))?;
    }
    tx.commit().map_err(|_| AppError::storage("无法保存标签"))?;
    message_by_id(conn, account_id, message_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_changes::tests::seed;

    fn fixture() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        conn
    }

    #[test]
    fn create_validates_name_color_and_duplicates() {
        let conn = fixture();
        let error = create(&conn, "  ", "teal").unwrap_err();
        assert_eq!(error.code, "invalid");
        let error = create(&conn, "工作", "purple").unwrap_err();
        assert_eq!(error.code, "invalid");
        let long = "标".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(create(&conn, &long, "teal").unwrap_err().code, "invalid");
        let label = create(&conn, "  工作  ", "amber").unwrap();
        assert!(label.id.starts_with(USER_LABEL_PREFIX));
        assert_eq!(label.name, "工作");
        assert_eq!(label.scope, "local");
        assert!(label.account_id.is_none());
        // 同名（trim 后）拒绝；不同名可建。
        assert_eq!(create(&conn, "工作", "teal").unwrap_err().code, "conflict");
        assert!(create(&conn, "项目", "blue").is_ok());
    }

    #[test]
    fn rename_updates_name_and_optional_color_with_guards() {
        let conn = fixture();
        let label = create(&conn, "工作", "teal").unwrap();
        let other = create(&conn, "项目", "blue").unwrap();
        // 不存在 / 服务器 scope（守卫外）→ 拒绝。
        assert_eq!(
            rename(&conn, "missing", "x", None).unwrap_err().code,
            "invalid"
        );
        conn.execute(
            "INSERT INTO labels(id, account_id, remote_id, name, color, scope) VALUES ('srv','acc','remote','Remote','teal','server')",
            [],
        )
        .unwrap();
        assert_eq!(rename(&conn, "srv", "x", None).unwrap_err().code, "invalid");
        // 重名拒绝；改名成功；仅改色成功。
        assert_eq!(
            rename(&conn, &label.id, "项目", None).unwrap_err().code,
            "conflict"
        );
        let renamed = rename(&conn, &label.id, "工作流", Some("amber")).unwrap();
        assert_eq!(renamed.name, "工作流");
        assert_eq!(renamed.color, "amber");
        assert_eq!(
            rename(&conn, &other.id, "项目", None).unwrap().color,
            "blue"
        );
        // 颜色非法 → 拒绝且不改名。
        assert_eq!(
            rename(&conn, &other.id, "项目x", Some("red"))
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(label_row(&conn, &other.id).unwrap().name, "项目");
    }

    #[test]
    fn delete_removes_row_and_message_links_cascade() {
        let conn = fixture();
        let label = create(&conn, "工作", "teal").unwrap();
        conn.execute(
            "INSERT INTO message_labels VALUES ('m', ?1)",
            params![&label.id],
        )
        .unwrap();
        delete(&conn, &label.id).unwrap();
        assert_eq!(delete(&conn, &label.id).unwrap_err().code, "invalid");
        let links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_labels WHERE label_id=?1",
                params![&label.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(links, 0, "删除标签应级联清理邮件关联");
    }

    #[test]
    fn set_for_message_replaces_set_and_validates_everything() {
        let mut conn = fixture();
        let a = create(&conn, "工作", "teal").unwrap();
        let b = create(&conn, "项目", "blue").unwrap();
        // 未知标签 / 未知邮件 / 账号不匹配 → 拒绝且不写入。
        assert_eq!(
            set_for_message(&mut conn, "acc", "m", &["nope".into()])
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(
            set_for_message(&mut conn, "acc", "missing", &[])
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(
            set_for_message(&mut conn, "other", "m", &[])
                .unwrap_err()
                .code,
            "invalid"
        );
        let ids = vec![a.id.clone(), b.id.clone(), a.id.clone()];
        let row = set_for_message(&mut conn, "acc", "m", &ids).unwrap();
        let mut got = row.label_ids.clone();
        got.sort();
        let mut want = vec![a.id.clone(), b.id.clone()];
        want.sort();
        assert_eq!(got, want, "去重（读取顺序按标签 id 字典序）");
        // 替换语义：再次设置只保留新集合。
        let ids = vec![b.id.clone()];
        let row = set_for_message(&mut conn, "acc", "m", &ids).unwrap();
        assert_eq!(row.label_ids, vec![b.id.clone()]);
        let row = set_for_message(&mut conn, "acc", "m", &[]).unwrap();
        assert!(row.label_ids.is_empty(), "空集合清除全部标签");
        // 状态修订不被标签修改推进（无回写语义）。
        let revision: i64 = conn
            .query_row(
                "SELECT state_revision FROM messages WHERE id='m'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(revision, 0);
    }
}
