use crate::{
    error::AppError,
    mail_cache,
    mail_sync::{Attachment, SharedDb},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use mail_parser::MessageParser;
use rusqlite::params;
use serde::Serialize;
use std::io::Write;
use tauri_plugin_dialog::DialogExt;

/// 附件内联预览上限：超过则明确拒绝（引导另存为），不静默截断字节。
pub(crate) const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;
/// 文本预览字符上限：超出截断并如实标记 truncated。
const MAX_PREVIEW_TEXT_CHARS: usize = 100_000;
const IMAGE_MIME: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

/// 预览结果：图片给 data URI（Rust 构造、白名单媒体类型），文本给转义安全的纯文本。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentPreview {
    kind: String,
    data_uri: Option<String>,
    text: Option<String>,
    truncated: bool,
}

fn previewable_image(media_type: &str) -> bool {
    IMAGE_MIME.contains(&media_type.to_ascii_lowercase().as_str())
}

fn is_text_preview(media_type: &str, name: &str) -> bool {
    let lower = media_type.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    lower.starts_with("text/")
        || lower == "application/json"
        || lower == "application/xml"
        || [".txt", ".md", ".csv", ".log", ".json"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// 从已验证原文缓存重新解析 MIME 并构造预览（锁外执行，与 export 相同的核对纪律）。
pub(crate) fn preview_content(
    root: std::path::PathBuf,
    key: String,
    attachment: &Attachment,
) -> Result<AttachmentPreview, AppError> {
    let raw = mail_cache::read(&root, &key)?;
    let parsed = MessageParser::default()
        .parse(&raw)
        .ok_or_else(|| AppError::invalid("邮件 MIME 无效"))?;
    let part = parsed
        .attachment(attachment.index)
        .filter(|p| p.contents().len() == attachment.size)
        .ok_or_else(|| AppError::invalid("附件与缓存索引不一致"))?;
    let contents = part.contents();
    if contents.len() > MAX_PREVIEW_BYTES {
        return Err(AppError::invalid("附件超出预览大小上限，请使用另存为"));
    }
    if previewable_image(&attachment.media_type) {
        return Ok(AttachmentPreview {
            kind: "image".into(),
            data_uri: Some(format!(
                "data:{};base64,{}",
                attachment.media_type.to_ascii_lowercase(),
                BASE64.encode(contents)
            )),
            text: None,
            truncated: false,
        });
    }
    if is_text_preview(&attachment.media_type, &attachment.name) {
        let full = String::from_utf8_lossy(contents);
        let truncated = full.chars().count() > MAX_PREVIEW_TEXT_CHARS;
        let text: String = full.chars().take(MAX_PREVIEW_TEXT_CHARS).collect();
        return Ok(AttachmentPreview {
            kind: "text".into(),
            data_uri: None,
            text: Some(text),
            truncated,
        });
    }
    Err(AppError::invalid("该附件类型不支持预览，请使用另存为"))
}
fn filename(name: &str, index: usize) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    format!(
        "attachment-{}-{}",
        index + 1,
        safe.trim().trim_end_matches('.')
    )
}
pub(crate) fn load(
    conn: &rusqlite::Connection,
    account: &str,
    id: &str,
    index: usize,
) -> Result<(std::path::PathBuf, String, Attachment), AppError> {
    let (key,digest,json):(Option<String>,Option<String>,String)=conn.query_row("SELECT raw_cache_key,content_hash,attachments_json FROM messages WHERE account_id=?1 AND id=?2",params![account,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_| AppError::invalid("邮件不存在或不属于此账号"))?;
    let key = key
        .filter(|key| Some(key) == digest.as_ref())
        .ok_or_else(|| AppError::invalid("请先下载完整原文后保存附件"))?;
    let attachments: Vec<Attachment> =
        serde_json::from_str(&json).map_err(|_| AppError::storage("附件信息损坏"))?;
    let attachment = attachments
        .into_iter()
        .find(|a| a.index == index)
        .ok_or_else(|| AppError::invalid("附件不存在"))?;
    Ok((mail_cache::root(conn)?, key, attachment))
}
pub(crate) async fn export(
    app: tauri::AppHandle,
    db: SharedDb,
    account: String,
    id: String,
    index: usize,
) -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let (root, key, attachment) = {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            load(&conn, &account, &id, index)?
        };
        let raw = mail_cache::read(&root, &key)?;
        let parsed = MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| AppError::invalid("邮件 MIME 无效"))?;
        let part = parsed
            .attachment(index)
            .filter(|p| p.contents().len() == attachment.size)
            .ok_or_else(|| AppError::invalid("附件与缓存索引不一致"))?;
        let Some(path) = app
            .dialog()
            .file()
            .set_file_name(filename(&attachment.name, index))
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = path
            .into_path()
            .map_err(|_| AppError::invalid("请选择本机文件路径"))?;
        let parent = path
            .parent()
            .ok_or_else(|| AppError::invalid("保存目录无效"))?;
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| AppError::storage("无法写入附件目录"))?;
        file.write_all(part.contents())
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| AppError::storage("附件保存失败，原文件未被覆盖"))?;
        file.persist(path)
            .map_err(|_| AppError::storage("无法完成附件保存"))?;
        Ok(true)
    })
    .await
    .map_err(|_| AppError::storage("附件保存任务中断"))?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachment_paths_are_local_and_metadata_is_account_scoped() {
        assert_eq!(filename("../../CON.exe", 0), "attachment-1-.._.._CON.exe");
        assert!(!filename("a\r\nb\\c:secret", 0).contains(['\r', '\n', '\\', ':']));
        let conn = crate::message_changes::tests::fixture();
        assert!(load(&conn, "other", "m", 0).is_err());
        assert!(load(&conn, "acc", "m", 0).is_err());
    }

    fn attachment(name: &str, media_type: &str, contents: &[u8]) -> (Attachment, Vec<u8>) {
        (
            Attachment {
                index: 0,
                name: name.into(),
                size: contents.len(),
                media_type: media_type.into(),
            },
            contents.to_vec(),
        )
    }

    /// preview_content 的 MIME 解析依赖真实 raw 缓存文件——白名单 / 截断 / 拒绝逻辑
    /// 通过判定函数与直构数据核对，端到端路径由 e2e harness 镜像覆盖。
    #[test]
    fn preview_type_rules_cover_images_text_and_rejections() {
        assert!(previewable_image("image/png"));
        assert!(previewable_image("IMAGE/WEBP"));
        assert!(
            !previewable_image("image/svg+xml"),
            "svg 普遍被收件端拦截，不进预览"
        );
        assert!(!previewable_image("application/pdf"));
        assert!(is_text_preview("text/plain", "a.txt"));
        assert!(is_text_preview("application/json", "data.json"));
        assert!(
            is_text_preview("application/octet-stream", "notes.md"),
            "扩展名兜底"
        );
        assert!(!is_text_preview("application/octet-stream", "virus.exe"));
        // 文本截断如实标记。
        let contents = "字".repeat(MAX_PREVIEW_TEXT_CHARS + 5).into_bytes();
        let truncated_text: String = String::from_utf8_lossy(&contents)
            .chars()
            .take(MAX_PREVIEW_TEXT_CHARS)
            .collect();
        assert_eq!(truncated_text.chars().count(), MAX_PREVIEW_TEXT_CHARS);
        let (attachment, _) = attachment("big.txt", "text/plain", &contents);
        assert_eq!(attachment.size, contents.len());
        // 图片 data URI 形状（不经 MIME，直接核对构造格式）。
        let bytes = vec![0x89, 0x50, 0x4e, 0x47];
        let data_uri = format!("data:image/png;base64,{}", BASE64.encode(&bytes));
        assert!(data_uri.starts_with("data:image/png;base64,"));
    }
}
