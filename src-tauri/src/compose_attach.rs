//! 撰写附件：本机文件选取与进程内暂存。
//! 内容只在 Rust 侧流转（对话框 → 暂存 → 保存事务写入 SQLite BLOB → 发送时读出），
//! 前端仅持有元数据；暂存未落库前进程退出即丢失，保存失败保留暂存供重试（不静默丢附件）。
use crate::{
    drafts::{valid_draft_id, DraftAttachment, MAX_ATTACH_COUNT, MAX_ATTACH_TOTAL},
    error::AppError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tauri_plugin_dialog::DialogExt;

/// 暂存区总字节上限（跨草稿合计）：保存 / 放弃流程会主动回收，超过时明确拒绝新选取。
const STAGING_LIMIT: usize = 64 * 1024 * 1024;

/// (draft_id, attachment_id) → 已选取未落库的内容。 managed state；进程重启即清空。
#[derive(Default, Clone)]
pub(crate) struct ComposeStaging(Arc<Mutex<HashMap<(String, String), Staged>>>);

struct Staged {
    sha256: String,
    content: Vec<u8>,
}

type StagingMap = HashMap<(String, String), Staged>;

impl ComposeStaging {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StagingMap>, AppError> {
        self.0
            .lock()
            .map_err(|_| AppError::storage("附件暂存不可用"))
    }

    fn total_bytes(map: &HashMap<(String, String), Staged>) -> usize {
        map.values().map(|staged| staged.content.len()).sum()
    }

    /// 暂存新附件内容；sha256 与元数据必须来自同一次读取。
    pub(crate) fn stage(
        &self,
        draft_id: &str,
        attachment: &DraftAttachment,
        content: Vec<u8>,
    ) -> Result<(), AppError> {
        let mut map = self.lock()?;
        if Self::total_bytes(&map) + content.len() > STAGING_LIMIT {
            return Err(AppError::storage(
                "附件暂存空间不足，请先保存当前草稿或重启应用后重试。",
            ));
        }
        map.insert(
            (draft_id.to_string(), attachment.id.clone()),
            Staged {
                sha256: attachment.sha256.clone(),
                content,
            },
        );
        Ok(())
    }

    /// 保存事务成功后回收该草稿的全部暂存（内容已落库，或附件已从列表移除）。
    pub(crate) fn consume_draft(&self, draft_id: &str) {
        if let Ok(mut map) = self.0.lock() {
            map.retain(|(owner, _), _| owner != draft_id);
        }
    }
}

/// 保存事务读取暂存内容：与元数据 sha256 一致才返回（不一致视为未暂存，由调用方报内容缺失）。
pub(crate) fn staged_content(
    staging: &ComposeStaging,
    draft_id: &str,
    attachment_id: &str,
    sha256: &str,
) -> Option<Vec<u8>> {
    let map = staging.lock().ok()?;
    let staged = map.get(&(draft_id.to_string(), attachment_id.to_string()))?;
    (staged.sha256 == sha256).then(|| staged.content.clone())
}

/// 常见扩展名 → MIME；不在表内的回退 application/octet-stream。
fn media_type_for(name: &str) -> String {
    const TABLE: &[(&str, &str)] = &[
        ("pdf", "application/pdf"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("svg", "image/svg+xml"),
        ("txt", "text/plain"),
        ("log", "text/plain"),
        ("md", "text/plain"),
        ("csv", "text/csv"),
        ("html", "text/html"),
        ("htm", "text/html"),
        ("json", "application/json"),
        ("xml", "application/xml"),
        ("doc", "application/msword"),
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        ("xls", "application/vnd.ms-excel"),
        (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        ("ppt", "application/vnd.ms-powerpoint"),
        (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        ("zip", "application/zip"),
        ("gz", "application/gzip"),
        ("7z", "application/x-7z-compressed"),
        ("mp3", "audio/mpeg"),
        ("wav", "audio/wav"),
        ("ogg", "audio/ogg"),
        ("mp4", "video/mp4"),
        ("mov", "video/quicktime"),
        ("ics", "text/calendar"),
        ("eml", "message/rfc822"),
    ];
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("")
        .to_ascii_lowercase();
    TABLE
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, media_type)| media_type.to_string())
        .unwrap_or_else(|| "application/octet-stream".into())
}

/// 文件名消毒：控制字符与路径符号替换为下划线，去掉结尾点，截断 240 字符。
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return "attachment.bin".into();
    }
    trimmed.chars().take(240).collect()
}

fn sha256_hex(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

/// 按给定元数据写入暂存：元数据 sha256 必须与内容一致（与保存侧核对规则相同）。
/// 仅供测试以固定 id 复现场景；生产路径统一经 `stage` 分配标识。
#[cfg(test)]
pub(crate) fn stage_existing(
    staging: &ComposeStaging,
    draft_id: &str,
    attachment: &DraftAttachment,
    content: Vec<u8>,
) -> Result<(), AppError> {
    if attachment.sha256 != sha256_hex(&content) || attachment.size != content.len() {
        return Err(AppError::invalid("附件内容与元数据不一致"));
    }
    staging.stage(draft_id, attachment, content)
}

/// 选取入口：分配 att- 标识、推导元数据并写入暂存。
pub(crate) fn stage(
    staging: &ComposeStaging,
    draft_id: &str,
    name: &str,
    content: Vec<u8>,
) -> Result<DraftAttachment, AppError> {
    let attachment = DraftAttachment {
        id: format!("att-{}", uuid::Uuid::new_v4().simple()),
        name: safe_name(name),
        size: content.len(),
        media_type: media_type_for(name),
        sha256: sha256_hex(&content),
    };
    staging.stage(draft_id, &attachment, content)?;
    Ok(attachment)
}

/// 撰写附件选取：Rust 侧多选文件对话框（前端不接触路径），读入暂存区后仅返回元数据。
/// 取消选择返回空列表（无操作，不伪装成功）。
pub(crate) async fn pick(
    staging: ComposeStaging,
    app: tauri::AppHandle,
    draft_id: String,
    current_total: usize,
    current_count: usize,
) -> Result<Vec<DraftAttachment>, AppError> {
    if !valid_draft_id(&draft_id) {
        return Err(AppError::invalid("草稿标识无效"));
    }
    if current_count >= MAX_ATTACH_COUNT {
        return Err(AppError::invalid("附件数量已达上限（每封邮件最多 20 个）"));
    }
    if current_total > MAX_ATTACH_TOTAL {
        return Err(AppError::invalid(
            "现有附件已超出大小限制，请先移除部分附件。",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let Some(paths) = app.dialog().file().blocking_pick_files() else {
            return Ok(Vec::new());
        };
        if paths.len() + current_count > MAX_ATTACH_COUNT {
            return Err(AppError::invalid("附件数量超出限制（每封邮件最多 20 个）"));
        }
        let mut picked = Vec::with_capacity(paths.len());
        let mut added = 0usize;
        for path in paths {
            let path = path
                .into_path()
                .map_err(|_| AppError::invalid("请选择本机文件。"))?;
            let raw_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            let declared = std::fs::metadata(&path)
                .map_err(|_| AppError::storage(format!("无法读取所选文件「{raw_name}」")))?
                .len() as usize;
            if declared > MAX_ATTACH_TOTAL {
                return Err(AppError::invalid(format!(
                    "「{}」超过单封邮件附件 16 MiB 上限。",
                    safe_name(&raw_name)
                )));
            }
            let content = std::fs::read(&path)
                .map_err(|_| AppError::storage(format!("无法读取所选文件「{raw_name}」")))?;
            if content.len() != declared {
                return Err(AppError::storage("文件在读取时发生了变化，请重试。"));
            }
            added += content.len();
            if current_total + added > MAX_ATTACH_TOTAL {
                return Err(AppError::invalid(
                    "附件总大小超出限制（16 MiB），请移除部分附件后重试。",
                ));
            }
            picked.push(stage(&staging, &draft_id, &raw_name, content)?);
        }
        Ok(picked)
    })
    .await
    .map_err(|_| AppError::storage("附件选取任务未完成"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_and_names_follow_extension_table_and_sanitization() {
        assert_eq!(media_type_for("报告.pdf"), "application/pdf");
        assert_eq!(media_type_for("A.PNG"), "image/png");
        assert_eq!(media_type_for("no-extension"), "application/octet-stream");
        assert_eq!(safe_name("a\r\nb\\c:secret.txt"), "a__b_c_secret.txt");
        assert_eq!(safe_name("  end.  "), "end");
        assert_eq!(safe_name("..."), "attachment.bin");
        assert_eq!(safe_name("中".repeat(300).as_str()).chars().count(), 240);
    }

    #[test]
    fn staging_roundtrip_consume_and_capacity() {
        let staging = ComposeStaging::default();
        let content = vec![7u8; 10];
        let attachment = stage(&staging, "draft-x", "note.txt", content.clone()).unwrap();
        assert_eq!(attachment.size, 10);
        assert_eq!(attachment.media_type, "text/plain");
        assert_eq!(
            staged_content(&staging, "draft-x", &attachment.id, &attachment.sha256).unwrap(),
            content
        );
        // sha 不一致时不返回内容（保存侧据此报内容缺失而不是写错字节）。
        assert!(staged_content(&staging, "draft-x", &attachment.id, "00").is_none());
        staging.consume_draft("draft-x");
        assert!(staged_content(&staging, "draft-x", &attachment.id, &attachment.sha256).is_none());
    }

    #[test]
    fn stage_rejects_beyond_staging_capacity() {
        let staging = ComposeStaging::default();
        let big = vec![1u8; STAGING_LIMIT];
        let staged = stage(&staging, "draft-big", "huge.bin", big);
        assert!(staged.is_ok());
        let another = stage(&staging, "draft-big", "more.bin", vec![2u8; 1]);
        assert_eq!(another.unwrap_err().code, "storage");
    }
}
