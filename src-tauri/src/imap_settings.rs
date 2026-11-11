use crate::error::AppError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settings {
    pub sync_all_folders: bool,
    pub history_enabled: bool,
    pub auto_sync_seconds: u32,
}
pub(crate) fn read(conn: &rusqlite::Connection, account: &str) -> Result<Settings, AppError> {
    conn.query_row("SELECT sync_all_folders,history_enabled,auto_sync_seconds FROM imap_settings WHERE account_id=?1",[account],|r|Ok(Settings{sync_all_folders:r.get(0)?,history_enabled:r.get(1)?,auto_sync_seconds:r.get(2)?})).optional().map(|v|v.unwrap_or_default()).map_err(|_|AppError::storage("无法读取 IMAP 同步设置"))
}
pub(crate) fn save(
    conn: &rusqlite::Connection,
    account: &str,
    value: &Settings,
) -> Result<(), AppError> {
    if value.auto_sync_seconds != 0 && !(60..=86400).contains(&value.auto_sync_seconds) {
        return Err(AppError::invalid(
            "自动同步间隔必须为 60–86400 秒，或 0 表示关闭",
        ));
    }
    let real:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1 AND provider='imap' AND COALESCE(json_extract(CASE WHEN json_valid(profile_json) THEN profile_json ELSE '{}' END,'$.status'),'')!='demo')",[account],|r|r.get(0)).map_err(|_|AppError::storage("无法读取账号"))?;
    if !real {
        return Err(AppError::invalid("请选择真实 IMAP 账号"));
    }
    conn.execute("INSERT INTO imap_settings(account_id,sync_all_folders,history_enabled,auto_sync_seconds) VALUES (?1,?2,?3,?4) ON CONFLICT(account_id) DO UPDATE SET sync_all_folders=excluded.sync_all_folders,history_enabled=excluded.history_enabled,auto_sync_seconds=excluded.auto_sync_seconds",params![account,value.sync_all_folders,value.history_enabled,value.auto_sync_seconds]).map_err(|_|AppError::storage("无法保存 IMAP 设置"))?;
    Ok(())
}
pub(crate) fn scope(folder: &str) -> String {
    format!("folder-{}", crate::mail_recovery::hash(folder.as_bytes()))
}
pub(crate) fn valid_scope(scope: &str, folder: Option<&str>) -> bool {
    ["inbox", "sent", "archive"].contains(&scope)
        || folder.is_some_and(|folder| scope == self::scope(folder))
}

/// Decode display names only. Protocol names and scope hashes always retain server bytes.
pub(crate) fn display_name(raw: &str) -> String {
    use base64::Engine;
    let decode = || -> Option<String> {
        let mut rest = raw;
        let mut out = String::new();
        while let Some(start) = rest.find('&') {
            out.push_str(&rest[..start]);
            rest = &rest[start + 1..];
            let end = rest.find('-')?;
            let encoded = &rest[..end];
            if encoded.is_empty() {
                out.push('&');
            } else {
                let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                    .decode(encoded.replace(',', "/"))
                    .ok()?;
                if bytes.len() % 2 != 0 {
                    return None;
                }
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                out.push_str(&String::from_utf16(&units).ok()?);
            }
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    };
    decode().unwrap_or_else(|| raw.into())
}
#[cfg(test)]
mod tests {
    #[test]
    fn modified_utf7_changes_only_the_display_and_bad_names_survive() {
        assert_eq!(super::display_name("&ZeVnLIqe-"), "日本語");
        assert_eq!(super::display_name("R&-D"), "R&D");
        assert_eq!(super::display_name("&bad"), "&bad");
        assert_eq!(super::display_name("项目"), "项目");
        assert_ne!(super::scope("&ZeVnLIqe-"), super::scope("日本語"));
    }
}
