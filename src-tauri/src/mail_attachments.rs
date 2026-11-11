use crate::{
    error::AppError,
    mail_cache,
    mail_sync::{Attachment, SharedDb},
};
use mail_parser::MessageParser;
use rusqlite::params;
use std::io::Write;
use tauri_plugin_dialog::DialogExt;
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
}
