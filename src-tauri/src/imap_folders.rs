//! 远端目录 CRUD（仅 IMAP 真实账号的自定义目录）：CREATE / RENAME / DELETE 在
//! 独立短会话上执行（不占用同步队列会话，也不在持锁时做网络 I/O），服务器确认后
//! 更新本机目录缓存并返回最新目录列表。标准目录（收件箱 / 已发送 / 草稿 / 归档 /
//! 废纸篓）只读，防止破坏同步与意图回写的目录假设。
use crate::{credentials, error::AppError, mail_store, mail_sync::SharedDb};
use rusqlite::{params, OptionalExtension};
/// IMAP 邮箱名的 modified UTF-7 编码（RFC 3501 §5.1.3）：非 ASCII 连续段按 UTF-16BE
/// 取码元、Base64 改用 `,` 替代 `/`、以 `&…-` 包裹；ASCII 原样，`&` 转 `&-`。
/// 2026-11-18 验收实测：GreenMail 拒绝原始 UTF-8 目录名的 LIST，非 ASCII 名称必须编码。
pub(crate) fn encode_mutf7(name: &str) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut out = String::with_capacity(name.len());
    let utf16: Vec<u16> = name.encode_utf16().collect();
    let mut index = 0;
    while index < utf16.len() {
        if (0x20..0x7f).contains(&utf16[index]) && utf16[index] != b'&' as u16 {
            out.push(utf16[index] as u8 as char);
            index += 1;
            continue;
        }
        if utf16[index] == b'&' as u16 {
            out.push_str("&-");
            index += 1;
            continue;
        }
        // 非 ASCII 连续段整段编码。
        let start = index;
        while index < utf16.len() && !((0x20..0x7f).contains(&utf16[index])) {
            index += 1;
        }
        let mut bytes = Vec::new();
        for unit in &utf16[start..index] {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        out.push('&');
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let group = (b0 << 16) | (b1 << 8) | b2;
            let chars = [
                (group >> 18) as usize & 63,
                (group >> 12) as usize & 63,
                (group >> 6) as usize & 63,
                group as usize & 63,
            ];
            let keep = match chunk.len() {
                3 => 4,
                2 => 3,
                _ => 2,
            };
            for position in chars.iter().take(keep) {
                out.push(B64[*position] as char);
            }
        }
        out.push('-');
    }
    out
}

/// modified UTF-7 解码（服务器返回的目录名 → 显示名）；非法序列返回 None 由调用方
/// 回退原始字符串（与同步列表的既有展示行为一致）。
#[cfg(test)]
pub(crate) fn decode_mutf7(encoded: &str) -> Option<String> {
    const B64: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut out = String::new();
    let bytes: Vec<char> = encoded.chars().collect();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != '&' {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        let end = bytes[index + 1..].iter().position(|c| *c == '-')? + index + 1;
        let body: String = bytes[index + 1..end].iter().collect();
        if body.is_empty() {
            // "&-" 是 '&' 的转义；跳过终止 '-' 本身。
            out.push('&');
            index = end + 1;
            continue;
        }
        let mut buffer = Vec::new();
        for group in body.as_bytes().chunks(4) {
            if group.len() < 2 {
                return None;
            }
            let mut values = [0u32; 4];
            for (slot, byte) in values.iter_mut().zip(group) {
                *slot = B64.find(*byte as char)? as u32;
            }
            let keep = match group.len() {
                4 => 3,
                3 => 2,
                _ => 1,
            };
            let combined = (values[0] << 18) | (values[1] << 12) | (values[2] << 6) | values[3];
            // 32 位容器最高字节恒 0（6-bit 字符拼出的值 ≤ 0xFFFFFF），真实字节在 [1..1+keep]，
            // 填充位在最后一个字节的低位侧。
            buffer.extend_from_slice(&combined.to_be_bytes()[1..1 + keep]);
        }
        out.push_str(&from_utf16be_pairs(&buffer)?);
        index = end + 1;
    }
    Some(out)
}

#[cfg(test)]
fn from_utf16be_pairs(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    String::from_utf16(
        &bytes
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .ok()
}

/// 名称消毒：去首尾空白、拒绝控制字符与过长名称（服务器仍会做 IMAP 层校验）。
fn validate_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > 200
        || trimmed.chars().any(|c| c.is_control())
    {
        return Err(AppError::invalid(
            "目录名称无效（1–200 字符，不含控制字符）",
        ));
    }
    Ok(trimmed.to_string())
}

/// 读取 IMAP 账号连接参数；非 IMAP 真实账号明确拒绝（POP3 没有目录概念）。
fn account_endpoints(
    conn: &rusqlite::Connection,
    account: &str,
) -> Result<(String, u16, crate::imap_transport::Security, String), AppError> {
    let (provider, profile): (String, Option<String>) = conn
        .query_row(
            "SELECT provider,profile_json FROM accounts WHERE id=?1",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取账号"))?
        .ok_or_else(|| AppError::invalid("账号不存在"))?;
    let profile: serde_json::Value = profile
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if provider != "imap" || profile["status"] == "demo" {
        return Err(AppError::invalid("目录管理仅支持真实的 IMAP 账号"));
    }
    let host = profile["imapHost"].as_str().unwrap_or_default().to_string();
    let port = profile["imapPort"].as_u64().unwrap_or(993) as u16;
    let security = match profile["imapSecurity"].as_str() {
        Some("starttls") => crate::imap_transport::Security::Starttls,
        _ => crate::imap_transport::Security::Tls,
    };
    let auth_user = profile["authUser"].as_str().unwrap_or_default().to_string();
    Ok((host, port, security, auth_user))
}

/// 独立短会话：不持有 DB 锁；失败映射为诚实的网络 / 认证错误。
fn open_session(
    db: &SharedDb,
    account: &str,
) -> Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>, AppError> {
    let (host, port, security, auth_user) = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        account_endpoints(&conn, account)?
    };
    let secret =
        credentials::with_account("imap", account, || credentials::secret_for("imap", account))?;
    let tls = crate::imap_transport::connect(&host, port, security)?;
    imap::Client::new(tls)
        .login(&auth_user, &secret)
        .map_err(|(error, _)| match error {
            imap::Error::No(_) | imap::Error::Bad(_) => {
                AppError::auth("IMAP 服务器拒绝了登录，请核对账号密码")
            }
            _ => crate::imap_limits::network(error, "IMAP 会话未能建立"),
        })
}

fn folder_list(
    conn: &rusqlite::Connection,
    account: &str,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT f.id,f.account_id,f.name,f.kind,COALESCE(s.available,1) FROM folders f \
             LEFT JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id \
             WHERE f.account_id=?1 ORDER BY f.name",
        )
        .map_err(|e| AppError::storage(e.to_string()))?;
    let rows = stmt
        .query_map(params![account], |row| {
            Ok(mail_store::FolderRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                available: row.get(4)?,
            })
        })
        .map_err(|e| AppError::storage(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::storage(e.to_string()))
}

/// 自定义目录行：kind 必须是 other（标准目录只读），且可用。
fn custom_folder(
    conn: &rusqlite::Connection,
    account: &str,
    folder_id: &str,
) -> Result<(String,), AppError> {
    let (name, kind, available): (String, String, bool) = conn
        .query_row(
            "SELECT name,kind,COALESCE(s.available,1) FROM folders f \
             LEFT JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id \
             WHERE f.id=?1 AND f.account_id=?2",
            params![folder_id, account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0)),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取目录"))?
        .ok_or_else(|| AppError::invalid("目录不存在"))?;
    if kind != "other" {
        return Err(AppError::invalid(
            "标准目录（收件箱 / 已发送 / 草稿 / 归档 / 废纸篓）不能修改",
        ));
    }
    if !available {
        return Err(AppError::invalid("目录当前不可用，无法修改"));
    }
    Ok((name,))
}

/// 本机缓存写入：remote_id / id 使用线格式（mUTF-7，与同步 LIST 存储的名称键一致，
/// 避免重名目录行）；name 存用户输入的显示名。
fn insert_local(
    conn: &mut rusqlite::Connection,
    account: &str,
    display_name: &str,
) -> Result<(), AppError> {
    let wire = encode_mutf7(display_name);
    let id = format!("{account}:folder:{wire}");
    let kind = crate::mail_sync::folder_kind(display_name);
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("目录缓存更新未完成"))?;
    tx.execute(
        "INSERT INTO folders (id,account_id,remote_id,name,kind) VALUES (?1,?2,?3,?4,?5) \
         ON CONFLICT(account_id,remote_id) DO UPDATE SET name=excluded.name,kind=excluded.kind",
        params![id, account, wire, display_name, kind],
    )
    .map_err(|_| AppError::storage("目录缓存写入失败"))?;
    tx.execute(
        "INSERT OR IGNORE INTO imap_folder_state (account_id,remote_id,available) VALUES (?1,?2,1)",
        params![account, wire],
    )
    .map_err(|_| AppError::storage("目录可用状态写入失败"))?;
    tx.commit()
        .map_err(|_| AppError::storage("目录缓存提交未完成"))
}

/// 新建远端目录。服务器确认后才写本机缓存；名称已存在时返回服务器错误。
pub fn create(
    db: &SharedDb,
    account: &str,
    name: &str,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let name = validate_name(name)?;
    let wire = encode_mutf7(&name);
    {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        account_endpoints(&conn, account)?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM folders WHERE account_id=?1 AND remote_id=?2)",
                params![account, wire],
                |row| row.get(0),
            )
            .map_err(|_| AppError::storage("无法核对目录名"))?;
        if exists {
            return Err(AppError::invalid("同名目录已存在"));
        }
    }
    let mut session = open_session(db, account)?;
    session
        .create(&wire)
        .map_err(|error| crate::imap_limits::network(error, "服务器拒绝了新建目录"))?;
    let _ = session.logout();
    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    insert_local(&mut conn, account, &name)?;
    folder_list(&conn, account)
}

/// 重命名自定义目录：服务器 RENAME 确认后删除旧目录行（级联清掉旧成员关联与可用
/// 状态）并写入新目录行；缓存邮件保留，下次同步按服务器归属重新关联。
pub fn rename(
    db: &SharedDb,
    account: &str,
    folder_id: &str,
    new_name: &str,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let new_name = validate_name(new_name)?;
    let old_name = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        account_endpoints(&conn, account)?;
        let (name,) = custom_folder(&conn, account, folder_id)?;
        if name == new_name {
            return Err(AppError::invalid("新名称与当前名称相同"));
        }
        name
    };
    let mut session = open_session(db, account)?;
    session
        .rename(encode_mutf7(&old_name), encode_mutf7(&new_name))
        .map_err(|error| crate::imap_limits::network(error, "服务器拒绝了目录重命名"))?;
    let _ = session.logout();
    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("目录缓存更新未完成"))?;
    tx.execute(
        "DELETE FROM folders WHERE id=?1 AND account_id=?2",
        params![folder_id, account],
    )
    .map_err(|_| AppError::storage("旧目录缓存清理失败"))?;
    tx.commit()
        .map_err(|_| AppError::storage("目录缓存提交未完成"))?;
    insert_local(&mut conn, account, &new_name)?;
    folder_list(&conn, account)
}

/// 删除自定义目录：服务器 DELETE 确认后删除本机目录行（级联成员关联）；缓存邮件
/// 保留（服务器上它们已随目录删除），同步游标按目录键独立，不会误判增量。
pub fn delete(
    db: &SharedDb,
    account: &str,
    folder_id: &str,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let (name,) = {
        let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        account_endpoints(&conn, account)?;
        custom_folder(&conn, account, folder_id)?
    };
    let mut session = open_session(db, account)?;
    session
        .delete(encode_mutf7(&name))
        .map_err(|error| crate::imap_limits::network(error, "服务器拒绝了删除目录"))?;
    let _ = session.logout();
    let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    let tx = conn
        .transaction()
        .map_err(|_| AppError::storage("目录缓存更新未完成"))?;
    tx.execute(
        "DELETE FROM folders WHERE id=?1 AND account_id=?2",
        params![folder_id, account],
    )
    .map_err(|_| AppError::storage("目录缓存清理失败"))?;
    tx.commit()
        .map_err(|_| AppError::storage("目录缓存提交未完成"))?;
    folder_list(&conn, account)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts(id,provider,email,display_name,profile_json,created_at) VALUES \
             ('acc','imap','t@example.test','T','{\"status\":\"offline\",\"imapHost\":\"127.0.0.1\",\"imapPort\":13143,\"imapSecurity\":\"tls\",\"authUser\":\"acc\"}','2026-11-18T00:00:00Z');
             INSERT INTO accounts(id,provider,email,display_name,profile_json,created_at) VALUES \
             ('p','pop3','p@example.test','P','{\"status\":\"offline\"}','2026-11-18T00:00:00Z');
             INSERT INTO folders VALUES ('in','acc','INBOX','INBOX','inbox');
             INSERT INTO folders VALUES ('c1','acc','Custom','Custom','other');
             INSERT INTO imap_folder_state VALUES ('acc','Custom',1);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn mutf7_roundtrips_ascii_ampersand_and_cjk() {
        // ASCII 原样；& 转义为 &-。
        assert_eq!(encode_mutf7("Projects/2026"), "Projects/2026");
        assert_eq!(encode_mutf7("a&b"), "a&-b");
        assert_eq!(decode_mutf7("a&-b").as_deref(), Some("a&b"));
        // 中文连续段整体编码，解码还原。
        let encoded = encode_mutf7("季度报告");
        assert!(encoded.starts_with('&') && encoded.ends_with('-'));
        assert_eq!(decode_mutf7(&encoded).as_deref(), Some("季度报告"));
        // ASCII 与中文相邻、多段切换。
        let mixed = encode_mutf7("邮件Mail2026");
        assert_eq!(decode_mutf7(&mixed).as_deref(), Some("邮件Mail2026"));
        // RFC 3501 规范对：é (U+00E9) ↔ &AOk-。
        let e9 = char::from_u32(0xe9).unwrap();
        assert_eq!(
            decode_mutf7("&AOk-").as_deref(),
            Some(e9.to_string().as_str())
        );
        assert_eq!(encode_mutf7(e9.to_string().as_str()), "&AOk-");
        // 非法 Base64 返回 None。
        assert_eq!(decode_mutf7("&!!!!-"), None);
    }

    #[test]
    fn validate_name_rejects_empty_control_and_overlong() {
        assert!(validate_name("  项目  ").unwrap() == "项目");
        assert!(validate_name("").is_err());
        assert!(validate_name("a\nb").is_err());
        assert!(validate_name(&"长".repeat(201)).is_err());
    }

    #[test]
    fn endpoints_require_real_imap_account() {
        let conn = fixture();
        // POP3 拒绝。
        let error = account_endpoints(&conn, "p").unwrap_err();
        assert_eq!(error.code, "invalid");
        // IMAP 返回连接参数。
        let (host, port, security, user) = account_endpoints(&conn, "acc").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 13143);
        assert_eq!(user, "acc");
        assert_eq!(security, crate::imap_transport::Security::Tls);
        // 不存在的账号。
        assert!(account_endpoints(&conn, "missing").is_err());
    }

    #[test]
    fn custom_folder_guard_protects_standard_and_unavailable() {
        let conn = fixture();
        // 标准目录拒绝。
        let error = custom_folder(&conn, "acc", "in").unwrap_err();
        assert_eq!(error.code, "invalid");
        // 自定义可用目录返回名称。
        assert_eq!(custom_folder(&conn, "acc", "c1").unwrap().0, "Custom");
        // 不可用目录拒绝。
        conn.execute(
            "UPDATE imap_folder_state SET available=0 WHERE remote_id='Custom'",
            [],
        )
        .unwrap();
        let error = custom_folder(&conn, "acc", "c1").unwrap_err();
        assert_eq!(error.code, "invalid");
        // 不存在的目录。
        assert!(custom_folder(&conn, "acc", "missing").is_err());
    }

    #[test]
    fn insert_local_uses_sync_kind_heuristics_and_marks_available() {
        let mut conn = fixture();
        insert_local(&mut conn, "acc", "项目邮件").unwrap();
        // remote_id 是线格式（mUTF-7），name 是显示名。
        let wire = encode_mutf7("项目邮件");
        assert_ne!(wire, "项目邮件");
        let (kind, available, stored_name): (String, i64, String) = conn
            .query_row(
                "SELECT f.kind,COALESCE(s.available,0),f.name FROM folders f \
                 LEFT JOIN imap_folder_state s ON s.account_id=f.account_id AND s.remote_id=f.remote_id \
                 WHERE f.account_id='acc' AND f.remote_id=?1",
                params![wire],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "other");
        assert_eq!(available, 1);
        assert_eq!(stored_name, "项目邮件");
        // 名称含 trash 字样时与同步分类一致。
        insert_local(&mut conn, "acc", "OldTrash").unwrap();
        let kind: String = conn
            .query_row(
                "SELECT kind FROM folders WHERE account_id='acc' AND remote_id='OldTrash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "trash");
    }
}
