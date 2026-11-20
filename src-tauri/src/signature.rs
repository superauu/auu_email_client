//! 每账号邮件签名（v23 多签名）：签名列表 + 每账号唯一默认签名（撰写注入用）。
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
/// 签名条目数上限（每账号）与名称长度上限。
pub(crate) const MAX_SIGNATURES_PER_ACCOUNT: usize = 10;
const MAX_NAME_CHARS: usize = 100;

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

/// 签名列表条目：id 为不透明字符串；is_default 供撰写注入与设置页星标。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignatureEntry {
    pub id: String,
    pub name: String,
    pub html: String,
    pub use_new: bool,
    pub use_reply: bool,
    pub is_default: bool,
}

fn real_account_guard(conn: &rusqlite::Connection, account: &str) -> Result<(), AppError> {
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
    Ok(())
}

/// 读取默认签名（撰写注入语义）：无默认条目时返回空默认值（不注入）。
/// 旧行为兼容：v23 迁移把既有单签名置为默认，因此既有账号注入行为不变。
pub(crate) fn read(conn: &rusqlite::Connection, account: &str) -> Result<Signature, AppError> {
    conn.query_row(
        "SELECT html,use_new,use_reply FROM signatures WHERE account_id=?1 AND is_default=1 \
         ORDER BY updated_at DESC, id LIMIT 1",
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

fn entry_from_row(row: &rusqlite::Row<'_>) -> Result<SignatureEntry, rusqlite::Error> {
    Ok(SignatureEntry {
        id: row.get(0)?,
        name: row.get(1)?,
        html: row.get(2)?,
        use_new: row.get(3)?,
        use_reply: row.get(4)?,
        is_default: row.get::<_, i64>(5)? != 0,
    })
}

/// 签名列表（按名称排序；默认条目由 is_default 标记）。
pub(crate) fn list(
    conn: &rusqlite::Connection,
    account: &str,
) -> Result<Vec<SignatureEntry>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT id,name,html,use_new,use_reply,is_default FROM signatures \
             WHERE account_id=?1 ORDER BY name, id",
        )
        .map_err(|_| AppError::storage("无法读取签名列表"))?;
    let entries = statement
        .query_map([account], entry_from_row)
        .map_err(|_| AppError::storage("无法读取签名列表"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("无法读取签名列表"))?;
    Ok(entries)
}

/// 新建或更新一条签名（id 缺省 = 新建）；make_default 时抢占该账号唯一默认。
/// 无任何默认签名时保存的第一条自动成为默认（保证撰写注入有明确语义）。
pub(crate) fn save_entry(
    conn: &mut rusqlite::Connection,
    account: &str,
    id: Option<&str>,
    name: &str,
    value: &Signature,
    make_default: bool,
) -> Result<SignatureEntry, AppError> {
    real_account_guard(conn, account)?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::invalid("签名名称为空或过长"));
    }
    if list(conn, account)?.len() >= MAX_SIGNATURES_PER_ACCOUNT && id.is_none() {
        return Err(AppError::invalid(
            "签名数量已达上限（每账号 10 条），请先删除不用的签名",
        ));
    }
    // 与草稿同一消毒策略（含 data:image/* 图片通道）；入库前消毒，读取方不必再处理。
    let html = crate::mail_html::sanitize_compose(&value.html);
    if html.encode_utf16().count() > MAX_BODY_HTML {
        return Err(AppError::invalid(
            "签名内容超出长度限制（含图片编码），请精简或压缩图片。",
        ));
    }
    let entry_id = match id {
        Some(existing) => {
            let exists: Option<String> = conn
                .query_row(
                    "SELECT id FROM signatures WHERE id=?1 AND account_id=?2",
                    params![existing, account],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| AppError::storage("无法核对签名"))?;
            if exists.is_none() {
                return Err(AppError::invalid("签名不存在或不属于此账号"));
            }
            existing.to_string()
        }
        None => format!("sig:{}", uuid::Uuid::new_v4()),
    };
    let updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("无法保存签名"))?;
    tx.execute(
        "INSERT INTO signatures(id,account_id,name,html,use_new,use_reply,is_default,updated_at) \
         VALUES (?1,?2,?3,?4,?5,?6,0,?7) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name,html=excluded.html,use_new=excluded.use_new,\
         use_reply=excluded.use_reply,updated_at=excluded.updated_at",
        params![entry_id, account, name, html, value.use_new, value.use_reply, updated_at],
    )
    .map_err(|_| AppError::storage("无法保存签名设置"))?;
    let should_default = make_default
        || tx
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM signatures WHERE account_id=?1 AND is_default=1)",
                [account],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| AppError::storage("无法核对默认签名"))?;
    if should_default {
        tx.execute(
            "UPDATE signatures SET is_default=0 WHERE account_id=?1",
            [account],
        )
        .map_err(|_| AppError::storage("无法更新默认签名"))?;
        tx.execute(
            "UPDATE signatures SET is_default=1 WHERE id=?1",
            [&entry_id],
        )
        .map_err(|_| AppError::storage("无法更新默认签名"))?;
    }
    tx.commit().map_err(|_| AppError::storage("无法保存签名"))?;
    let entry = conn
        .query_row(
            "SELECT id,name,html,use_new,use_reply,is_default FROM signatures WHERE id=?1",
            [&entry_id],
            entry_from_row,
        )
        .map_err(|_| AppError::storage("无法读取签名"))?;
    Ok(entry)
}

/// 删除签名；删除默认签名后不自动指派新默认（撰写注入自然回退为不注入）。
pub(crate) fn delete_entry(
    conn: &rusqlite::Connection,
    account: &str,
    id: &str,
) -> Result<(), AppError> {
    let deleted = conn
        .execute(
            "DELETE FROM signatures WHERE id=?1 AND account_id=?2",
            params![id, account],
        )
        .map_err(|_| AppError::storage("无法删除签名"))?;
    if deleted == 0 {
        return Err(AppError::invalid("签名不存在或不属于此账号"));
    }
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

    fn value(html: &str, use_new: bool, use_reply: bool) -> Signature {
        Signature {
            html: html.into(),
            use_new,
            use_reply,
        }
    }

    #[test]
    fn default_signature_semantics_survive_multiple_entries() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        // 无签名 = 空默认（不注入）。
        assert!(read(&conn, "real").unwrap().html.is_empty());
        // 第一条保存自动成为默认（无需显式 make_default）。
        let first = save_entry(
            &mut conn,
            "real",
            None,
            "工作",
            &value("<p>张三 · 工作签名</p>", true, false),
            false,
        )
        .unwrap();
        assert!(first.is_default);
        let injected = read(&conn, "real").unwrap();
        assert!(injected.html.contains("工作签名"));
        assert!(injected.use_new && !injected.use_reply, "开关随默认条目");
        // 第二条不抢占默认。
        let second = save_entry(
            &mut conn,
            "real",
            None,
            "个人",
            &value("<p>个人签名</p>", true, true),
            false,
        )
        .unwrap();
        assert!(!second.is_default);
        assert!(read(&conn, "real").unwrap().html.contains("工作签名"));
        // 显式设为默认后注入切换。
        save_entry(
            &mut conn,
            "real",
            Some(&second.id),
            "个人",
            &value("<p>个人签名</p>", true, true),
            true,
        )
        .unwrap();
        assert!(read(&conn, "real").unwrap().html.contains("个人签名"));
        // 列表含两条且默认唯一。
        let entries = list(&conn, "real").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().filter(|entry| entry.is_default).count(), 1);
        // 删除默认签名 → 回退为空默认（不自动指派）。
        delete_entry(&conn, "real", &second.id).unwrap();
        assert!(read(&conn, "real").unwrap().html.is_empty());
        assert!(!list(&conn, "real").unwrap().is_empty());
        // 更新消毒：data:image 保留、脚本剥除、演示账号拒绝。
        let saved = save_entry(
            &mut conn,
            "real",
            None,
            "带图",
            &value(
                "<p>张三</p><img src=\"data:image/png;base64,iVBORw0KGgo=\" alt=\"logo\"><script>bad()</script>",
                true,
                true,
            ),
            false,
        )
        .unwrap();
        assert!(saved.html.contains("data:image/png;base64,iVBORw0KGgo="));
        assert!(!saved.html.contains("<script"));
        assert!(saved.is_default, "此时账号无默认，自动指派");
        assert!(save_entry(&mut conn, "demo", None, "x", &value("h", true, true), false).is_err());
        assert!(save_entry(
            &mut conn,
            "missing",
            None,
            "x",
            &value("h", true, true),
            false
        )
        .is_err());
        // 跨账号 / 不存在的条目 id 拒绝更新。
        assert!(save_entry(
            &mut conn,
            "real",
            Some("sig:nope"),
            "x",
            &value("h", true, true),
            false
        )
        .is_err());
        assert!(delete_entry(&conn, "real", "sig:nope").unwrap_err().code == "invalid");
    }

    #[test]
    fn entry_name_and_count_limits_are_enforced() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        assert!(save_entry(
            &mut conn,
            "real",
            None,
            "   ",
            &value("h", true, true),
            false
        )
        .is_err());
        assert!(save_entry(
            &mut conn,
            "real",
            None,
            "名".repeat(MAX_NAME_CHARS + 1).as_str(),
            &value("h", true, true),
            false
        )
        .is_err());
        for index in 0..MAX_SIGNATURES_PER_ACCOUNT {
            save_entry(
                &mut conn,
                "real",
                None,
                &format!("签名{index}"),
                &value("h", true, true),
                false,
            )
            .unwrap();
        }
        assert_eq!(
            save_entry(
                &mut conn,
                "real",
                None,
                "第十一条",
                &value("h", true, true),
                false
            )
            .unwrap_err()
            .code,
            "invalid"
        );
        // 更新既有条目不受数量上限约束。
        let entries = list(&conn, "real").unwrap();
        save_entry(
            &mut conn,
            "real",
            Some(&entries[0].id),
            "改名",
            &value("h2", true, true),
            false,
        )
        .unwrap();
    }

    #[test]
    fn signature_html_over_limit_is_rejected() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        seed(&conn);
        assert!(save_entry(
            &mut conn,
            "real",
            None,
            "超长",
            &value(&format!("<p>{}</p>", "x".repeat(MAX_BODY_HTML)), true, true),
            false
        )
        .is_err());
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
