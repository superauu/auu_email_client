//! 远程图片信任列表（v21 迁移）：信任发件地址或发件域名后，阅读区自动加载其远程图片。
//! 全局作用域（发件人身份与接收账号无关）；条目入库前小写归一并校验形状。
use crate::error::AppError;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedEntry {
    /// 'sender'（完整发件地址）或 'domain'（发件域名，精确匹配不含子域）。
    pub kind: String,
    pub value: String,
    pub created_at: String,
}

fn validate(kind: &str, value: &str) -> Result<(), AppError> {
    let ok = match kind {
        // 发件地址：本地@域，宽松形状校验（与收信解析得到的地址归一后一致）。
        "sender" => {
            let parts: Vec<&str> = value.split('@').collect();
            parts.len() == 2
                && !parts[0].is_empty()
                && !parts[1].is_empty()
                && value.len() <= 320
                && value.bytes().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b'@' | b'.' | b'-' | b'_' | b'+')
                })
        }
        // 域名：点分标签，每段字母数字/连字符、不以连字符开头结尾。
        "domain" => {
            !value.is_empty()
                && value.len() <= 253
                && value.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                })
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid("信任条目格式无效"))
    }
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<TrustedEntry>, AppError> {
    let mut statement = conn
        .prepare("SELECT kind,value,created_at FROM trusted_senders ORDER BY created_at,value")
        .map_err(|_| AppError::storage("无法读取信任列表"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(TrustedEntry {
                kind: row.get(0)?,
                value: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|_| AppError::storage("无法读取信任列表"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("无法读取信任列表"))
}

pub(crate) fn add(conn: &Connection, kind: &str, value: &str) -> Result<TrustedEntry, AppError> {
    let kind = kind.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    validate(&kind, &value)?;
    let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    conn.execute(
        "INSERT INTO trusted_senders(kind,value,created_at) VALUES (?1,?2,?3) \
         ON CONFLICT(kind,value) DO NOTHING",
        params![kind, value, created_at],
    )
    .map_err(|_| AppError::storage("无法保存信任条目"))?;
    Ok(TrustedEntry {
        kind,
        value,
        created_at,
    })
}

pub(crate) fn remove(conn: &Connection, kind: &str, value: &str) -> Result<(), AppError> {
    let kind = kind.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    validate(&kind, &value)?;
    conn.execute(
        "DELETE FROM trusted_senders WHERE kind=?1 AND value=?2",
        params![kind, value],
    )
    .map_err(|_| AppError::storage("无法移除信任条目"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn trust_entries_roundtrip_dedupe_and_validate() {
        let conn = db();
        assert!(list(&conn).unwrap().is_empty());
        add(&conn, "Sender", "Azure@PromoMall.Microsoft.com ").unwrap();
        add(&conn, "domain", "Microsoft.com").unwrap();
        let entries = list(&conn).unwrap();
        assert_eq!(entries.len(), 2);
        // 小写归一 + trim。
        assert_eq!(entries[0].value, "azure@promomall.microsoft.com");
        assert_eq!(entries[0].kind, "sender");
        // 重复添加不产生第二行（幂等）。
        add(&conn, "sender", "azure@promomall.microsoft.com").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 2);
        remove(&conn, "sender", "azure@promomall.microsoft.com").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(list(&conn).unwrap()[0].value, "microsoft.com");
    }

    #[test]
    fn trust_validation_rejects_malformed_entries() {
        let conn = db();
        // 发件地址：缺 @、双 @、非法字符。
        assert!(add(&conn, "sender", "no-at-sign").is_err());
        assert!(add(&conn, "sender", "a@@b.test").is_err());
        assert!(add(&conn, "sender", "bad<script>@x.test").is_err());
        // 域名：空标签、连字符开头、超长。
        assert!(add(&conn, "domain", "bad..domain").is_err());
        assert!(add(&conn, "domain", "-lead.example").is_err());
        assert!(add(&conn, "domain", &"a".repeat(64)).is_err());
        // 未知 kind。
        assert!(add(&conn, "email", "x@y.test").is_err());
        assert!(list(&conn).unwrap().is_empty());
    }
}
