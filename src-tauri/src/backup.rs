//! 数据备份与恢复：导出 = VACUUM INTO 快照 + 原文缓存目录复制；恢复 = staging 后
//! 重启生效（不热切换全局连接）。系统凭据（密码 / OAuth refresh token / AI Key）
//! 不在备份内——恢复后账号需重新授权或重输密码，由前端确认文案明确说明。
//! 恢复采用 pending 标记：staging 只把备份放到暂存位置，下一次启动在 storage::
//! initialize 之前原子换库（当前库先备份为 mail.db.pre-restore）；失败保留 pending，
//! 不静默换库。
use crate::error::AppError;
use rusqlite::params;
use serde::Serialize;
use std::fs;
use std::path::Path;

const MARKER_FILE: &str = "restore.pending";
const PENDING_DB: &str = "mail.db.pending";
const PENDING_CACHE: &str = "mail-cache.pending";
const PRE_RESTORE_DB: &str = "mail.db.pre-restore";

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub cache_files: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreStaged {
    pub db_bytes: u64,
    pub cache_files: usize,
}

fn copy_dir_contents(source: &Path, target: &Path) -> Result<usize, AppError> {
    if !source.exists() {
        return Ok(0);
    }
    fs::create_dir_all(target).map_err(|_| AppError::storage("无法创建备份缓存目录"))?;
    let mut count = 0;
    let entries = fs::read_dir(source).map_err(|_| AppError::storage("无法读取缓存目录"))?;
    for entry in entries {
        let entry = entry.map_err(|_| AppError::storage("无法读取缓存目录"))?;
        let path = entry.path();
        if path.is_file() {
            fs::copy(&path, target.join(entry.file_name()))
                .map_err(|_| AppError::storage("缓存文件复制失败"))?;
            count += 1;
        }
    }
    Ok(count)
}

/// 导出：VACUUM INTO 快照（命令层持 DB 锁调用，保证一致性）+ 缓存目录复制
/// （调用方持有缓存全局锁，避免与同步写入竞争）。
pub(crate) fn export_snapshot(
    conn: &rusqlite::Connection,
    data_dir: &Path,
    target_dir: &Path,
) -> Result<ExportSummary, AppError> {
    let db_target = target_dir.join("mail.db");
    if db_target.exists() {
        return Err(AppError::invalid("目标目录已存在 mail.db，请选择空目录"));
    }
    conn.execute(
        "VACUUM INTO ?1",
        params![db_target.to_string_lossy().to_string()],
    )
    .map_err(|_| AppError::storage("数据库快照导出失败"))?;
    let cache_files =
        copy_dir_contents(&data_dir.join("mail-cache"), &target_dir.join("mail-cache"))?;
    Ok(ExportSummary { cache_files })
}

/// 恢复 staging：校验备份目录并复制到 data_dir 的 pending 位置 + 写标记。
pub(crate) fn stage_restore(data_dir: &Path, source_dir: &Path) -> Result<RestoreStaged, AppError> {
    let source_db = source_dir.join("mail.db");
    let db_bytes = fs::metadata(&source_db)
        .map_err(|_| AppError::invalid("备份目录中没有 mail.db，不是有效的备份"))?
        .len();
    if db_bytes == 0 {
        return Err(AppError::invalid("备份 mail.db 为空，不是有效的备份"));
    }
    fs::copy(&source_db, data_dir.join(PENDING_DB))
        .map_err(|_| AppError::storage("备份库复制失败"))?;
    let pending_cache = data_dir.join(PENDING_CACHE);
    if pending_cache.exists() {
        fs::remove_dir_all(&pending_cache).map_err(|_| AppError::storage("旧暂存缓存清理失败"))?;
    }
    let cache_files = copy_dir_contents(&source_dir.join("mail-cache"), &pending_cache)?;
    fs::write(data_dir.join(MARKER_FILE), crate::now_utc())
        .map_err(|_| AppError::storage("恢复标记写入失败"))?;
    Ok(RestoreStaged {
        db_bytes,
        cache_files,
    })
}

/// 启动时消费 pending 恢复。返回是否发生了恢复；失败返回 Err（保留 pending），
/// 调用方据此让启动失败——恢复失败绝不能静默继续用旧库。
pub(crate) fn consume_pending_restore(data_dir: &Path) -> Result<bool, String> {
    let marker = data_dir.join(MARKER_FILE);
    if !marker.exists() {
        return Ok(false);
    }
    let pending_db = data_dir.join(PENDING_DB);
    if !pending_db.exists() {
        let _ = fs::remove_file(&marker);
        return Err("恢复标记存在但暂存备份库缺失，已取消本次恢复".into());
    }
    let db = data_dir.join("mail.db");
    if db.exists() {
        fs::rename(&db, data_dir.join(PRE_RESTORE_DB))
            .map_err(|error| format!("当前数据库重命名失败：{error}"))?;
    }
    fs::rename(&pending_db, &db).map_err(|error| {
        // 换库失败：尽力把旧库还原回来，保留 pending 供人工处理。
        let pre = data_dir.join(PRE_RESTORE_DB);
        if pre.exists() {
            let _ = fs::rename(&pre, &db);
        }
        format!("暂存备份库换入失败：{error}")
    })?;
    let pending_cache = data_dir.join(PENDING_CACHE);
    if pending_cache.exists() {
        let cache = data_dir.join("mail-cache");
        fs::create_dir_all(&cache).map_err(|error| format!("缓存目录创建失败：{error}"))?;
        let entries =
            fs::read_dir(&pending_cache).map_err(|error| format!("暂存缓存读取失败：{error}"))?;
        for entry in entries.flatten() {
            let target = cache.join(entry.file_name());
            if target.exists() {
                fs::remove_file(&target).map_err(|error| format!("缓存文件替换失败：{error}"))?;
            }
            fs::rename(entry.path(), target)
                .map_err(|error| format!("缓存文件换入失败：{error}"))?;
        }
        fs::remove_dir_all(&pending_cache).map_err(|error| format!("暂存缓存清理失败：{error}"))?;
    }
    fs::remove_file(&marker).map_err(|error| format!("恢复标记清理失败：{error}"))?;
    Ok(true)
}

/// 恢复是否已 staged（等待重启生效）。
pub(crate) fn restore_pending(data_dir: &Path) -> bool {
    data_dir.join(MARKER_FILE).exists()
}

pub(crate) fn cache_lock_shared() -> &'static std::sync::Mutex<()> {
    &crate::mail_cache::CACHE_LOCK
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn seeded_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("auumail-backup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let conn = crate::storage::initialize(&dir.join("mail.db")).unwrap();
        crate::message_changes::tests::seed(&conn);
        let cache = dir.join("mail-cache");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("a".repeat(64)), b"raw-one").unwrap();
        fs::write(cache.join("b".repeat(64)), b"raw-two").unwrap();
        drop(conn);
        dir
    }

    #[test]
    fn export_snapshot_copies_db_and_cache_and_rejects_existing_target() {
        let data = seeded_dir("export");
        let target =
            std::env::temp_dir().join(format!("auumail-backup-target-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).unwrap();
        let conn = crate::storage::initialize(&data.join("mail.db")).unwrap();
        let summary = export_snapshot(&conn, &data, &target).unwrap();
        assert_eq!(summary.cache_files, 2);
        assert!(target.join("mail.db").exists());
        assert_eq!(
            fs::read(target.join("mail-cache").join("a".repeat(64))).unwrap(),
            b"raw-one"
        );
        // 目标已有 mail.db → 拒绝（不覆盖用户文件）。
        let error = export_snapshot(&conn, &data, &target).unwrap_err();
        assert_eq!(error.code, "invalid");
        drop(conn);
        let _ = fs::remove_dir_all(&data);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn restore_stage_and_consume_swaps_db_and_cache_atomically() {
        let data = seeded_dir("restore-data");
        let backup = seeded_dir("restore-backup");
        // 备份库加一张可辨识的表内容差异：改 seed 邮件主题。
        {
            let conn = crate::storage::initialize(&backup.join("mail.db")).unwrap();
            conn.execute("UPDATE messages SET subject='备份版主题' WHERE id='m'", [])
                .unwrap();
        }
        assert!(!restore_pending(&data));
        let staged = stage_restore(&data, &backup).unwrap();
        assert!(staged.db_bytes > 0);
        assert_eq!(staged.cache_files, 2);
        assert!(restore_pending(&data));
        // 启动消费：换库 + 合并缓存 + 标记清除；旧库保留为 pre-restore。
        assert!(consume_pending_restore(&data).unwrap());
        let conn = crate::storage::initialize(&data.join("mail.db")).unwrap();
        let subject: String = conn
            .query_row("SELECT subject FROM messages WHERE id='m'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(subject, "备份版主题");
        assert_eq!(
            fs::read(data.join("mail-cache").join("b".repeat(64))).unwrap(),
            b"raw-two"
        );
        assert!(data.join(PRE_RESTORE_DB).exists());
        assert!(!restore_pending(&data));
        drop(conn);
        // 再次 consume 是空操作。
        assert!(!consume_pending_restore(&data).unwrap());
        let _ = fs::remove_dir_all(&data);
        let _ = fs::remove_dir_all(&backup);
    }

    #[test]
    fn consume_without_staged_db_cancels_and_clears_marker() {
        let data = seeded_dir("orphan");
        fs::write(data.join(MARKER_FILE), "now").unwrap();
        let error = consume_pending_restore(&data).unwrap_err();
        assert!(error.contains("已取消本次恢复"));
        assert!(!restore_pending(&data), "孤儿标记应被清理，不阻塞后续启动");
        let _ = fs::remove_dir_all(&data);
    }
}
