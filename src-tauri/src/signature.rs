//! 每账号邮件签名（Outlook 式自动追加，v20 迁移）。
//! 签名 HTML 经 compose 消毒后入库，图片以 `data:image/*` 内嵌——发送时由 outbox
//! 转 `cid:` 内联件（multipart/related）；保存核对真实账号，演示账号与超限内容明确拒绝。
use crate::{drafts::MAX_BODY_HTML, error::AppError};
use base64::Engine;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri_plugin_dialog::DialogExt;

/// 签名单图上限（解码后字节）。base64 后约 1.37 MB，叠加签名文字与 body_html
/// 2M 上限同量级；超出明确拒绝，不做缩放（避免静默改图）。
pub(crate) const MAX_SIGNATURE_IMAGE: usize = 1024 * 1024;

/// 签名图片允许的位图类型（扩展名 → MIME）。排除 svg：收件端普遍拦截且并非位图。
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Signature {
    pub html: String,
    pub use_new: bool,
    pub use_reply: bool,
}

impl Default for Signature {
    fn default() -> Self {
        Self {
            html: String::new(),
            use_new: true,
            use_reply: true,
        }
    }
}

pub(crate) fn read(conn: &rusqlite::Connection, account: &str) -> Result<Signature, AppError> {
    conn.query_row(
        "SELECT html,use_new,use_reply FROM account_signatures WHERE account_id=?1",
        [account],
        |row| {
            Ok(Signature {
                html: row.get(0)?,
                use_new: row.get(1)?,
                use_reply: row.get(2)?,
            })
        },
    )
    .optional()
    .map(|value| value.unwrap_or_default())
    .map_err(|_| AppError::storage("无法读取签名设置"))
}

pub(crate) fn save(
    conn: &rusqlite::Connection,
    account: &str,
    value: &Signature,
) -> Result<(), AppError> {
    // 真实账号守卫：演示账号与不存在的账号都拒绝（与 imap_settings 同一判定）。
    let exists: Option<String> = conn
        .query_row(
            "SELECT id FROM accounts WHERE id=?1 AND COALESCE(json_extract(CASE WHEN \
             json_valid(profile_json) THEN profile_json ELSE '{}' END,'$.status'),'')!='demo'",
            [account],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取账号"))?;
    if exists.is_none() {
        return Err(AppError::invalid("请选择真实邮箱账号"));
    }
    // 与草稿同一消毒策略（含 data:image/* 图片通道）；入库前消毒，读取方不必再处理。
    let html = crate::mail_html::sanitize_compose(&value.html);
    if html.encode_utf16().count() > MAX_BODY_HTML {
        return Err(AppError::invalid(
            "签名内容超出长度限制（含图片编码），请精简或压缩图片。",
        ));
    }
    conn.execute(
        "INSERT INTO account_signatures(account_id,html,use_new,use_reply,updated_at) \
         VALUES (?1,?2,?3,?4,?5) \
         ON CONFLICT(account_id) DO UPDATE SET html=excluded.html,use_new=excluded.use_new,\
         use_reply=excluded.use_reply,updated_at=excluded.updated_at",
        params![
            account,
            html,
            value.use_new,
            value.use_reply,
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
        ],
    )
    .map_err(|_| AppError::storage("无法保存签名设置"))?;
    Ok(())
}

/// 签名图片选取结果：data URI 直接进编辑器（最终存入签名 HTML，发送时转 cid:）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickedImage {
    pub data_uri: String,
    pub name: String,
    pub size: usize,
}

fn image_media_type(name: &str) -> Option<&'static str> {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("")
        .to_ascii_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, media_type)| *media_type)
}

/// 签名图片选取：Rust 侧单选文件对话框，读取后返回 data URI（图片字节仅此一次过 IPC，
/// 随签名 HTML 保存；与草稿附件的暂存机制不同——签名图片是签名内容本身的一部分）。
/// 取消返回 None（无操作，不伪装成功）。
pub(crate) async fn pick_image(app: tauri::AppHandle) -> Result<Option<PickedImage>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app.dialog().file().blocking_pick_file() else {
            return Ok(None);
        };
        let path = path
            .into_path()
            .map_err(|_| AppError::invalid("请选择本机文件。"))?;
        let raw_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let Some(media_type) = image_media_type(&raw_name) else {
            return Err(AppError::invalid(
                "请选择 PNG / JPG / GIF / WebP / BMP 图片。",
            ));
        };
        let content = std::fs::read(&path)
            .map_err(|_| AppError::storage(format!("无法读取所选文件「{raw_name}」")))?;
        if content.len() > MAX_SIGNATURE_IMAGE {
            return Err(AppError::invalid(
                "图片超过 1 MiB 上限，请压缩后再插入签名。",
            ));
        }
        Ok(Some(PickedImage {
            data_uri: format!(
                "data:{media_type};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&content)
            ),
            name: raw_name,
            size: content.len(),
        }))
    })
    .await
    .map_err(|_| AppError::storage("图片选取任务未完成"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "INSERT INTO accounts (id,provider,email,display_name,created_at,profile_json) VALUES
      ('real','imap','real@example.test','Real','2026-11-19T00:00:00Z','{\"status\":\"connected\"}'),
      ('demo','imap','demo@example.test','Demo','2026-11-19T00:00:00Z','{\"status\":\"demo\"}');",
        )
        .unwrap();
    }

    #[test]
    fn signature_defaults_roundtrip_and_guards() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        // 无行 = 默认值（两个开关都开、空签名）。
        let initial = read(&conn, "real").unwrap();
        assert!(initial.html.is_empty() && initial.use_new && initial.use_reply);
        // 保存经 compose 消毒：data:image 保留、脚本剥除。
        let value = Signature {
            html: "<p>张三</p><img src=\"data:image/png;base64,iVBORw0KGgo=\" alt=\"logo\">\
                   <script>bad()</script>"
                .into(),
            use_new: true,
            use_reply: false,
        };
        save(&conn, "real", &value).unwrap();
        let saved = read(&conn, "real").unwrap();
        assert!(saved.html.contains("data:image/png;base64,iVBORw0KGgo="));
        assert!(saved.html.contains("张三"));
        assert!(!saved.html.contains("<script"));
        assert!(saved.use_new && !saved.use_reply);
        // 再次保存覆盖（upsert）。
        save(
            &conn,
            "real",
            &Signature {
                html: "v2".into(),
                ..Signature::default()
            },
        )
        .unwrap();
        assert_eq!(read(&conn, "real").unwrap().html, "v2");
        // 演示账号与不存在账号拒绝。
        assert!(save(&conn, "demo", &value).is_err());
        assert!(save(&conn, "missing", &value).is_err());
    }

    #[test]
    fn signature_html_over_limit_is_rejected() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        let value = Signature {
            html: format!("<p>{}</p>", "x".repeat(MAX_BODY_HTML)),
            ..Signature::default()
        };
        assert!(save(&conn, "real", &value).is_err());
    }

    #[test]
    fn signature_image_type_table_matches_policy() {
        assert_eq!(image_media_type("logo.PNG"), Some("image/png"));
        assert_eq!(image_media_type("photo.JPEG"), Some("image/jpeg"));
        assert_eq!(image_media_type("anim.gif"), Some("image/gif"));
        // svg 明确不允许（收件端拦截）。
        assert_eq!(image_media_type("icon.svg"), None);
        assert_eq!(image_media_type("script.exe"), None);
        assert_eq!(image_media_type("no-extension"), None);
    }
}
