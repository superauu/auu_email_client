//! Verified account + keyring persistence with compensation and generation checks.
use crate::{
    credentials,
    error::AppError,
    mail_store::{self, RealAccount},
    mail_sync::SharedDb,
};
use rusqlite::OptionalExtension;
pub(crate) fn configuration(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<RealAccount, AppError> {
    let (provider,email,name,profile): (String,String,String,String) = conn.query_row("SELECT provider,email,display_name,COALESCE(profile_json,'{}') FROM accounts WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|AppError::invalid("账号不存在"))?;
    let p: serde_json::Value =
        serde_json::from_str(&profile).map_err(|_| AppError::storage("账号配置损坏"))?;
    if provider != "imap" || p["status"] == "demo" {
        return Err(AppError::invalid("请选择真实 IMAP 账号"));
    }
    let value = serde_json::json!({"id":id,"name":name,"email":email,"imapHost":p["imapHost"],"imapPort":p["imapPort"],"imapSecurity":p.get("imapSecurity").cloned().unwrap_or(serde_json::json!("tls")),"smtpHost":p.get("smtpHost").cloned().unwrap_or(serde_json::json!("")),"smtpPort":p.get("smtpPort").cloned().unwrap_or(serde_json::json!(465)),"authUser":p["authUser"],"avatar":p["avatar"],"color":p["color"],"receiveOnly":p.get("receiveOnly").cloned().unwrap_or(serde_json::json!(false)),"oauth":p["oauth"]});
    serde_json::from_value(value).map_err(|_| AppError::storage("账号配置不完整，请重新配置"))
}
pub(crate) fn existing_generation(
    conn: &rusqlite::Connection,
    account: &RealAccount,
) -> Result<Option<String>, AppError> {
    if let Some(id) = &account.id {
        let row: Option<(String, String, Option<String>)> = conn
            .query_row(
                "SELECT provider,sync_generation,profile_json FROM accounts WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| AppError::storage("无法读取原账号"))?;
        let Some((provider, generation, profile)) = row else {
            return Err(AppError::invalid("原账号已不存在"));
        };
        if provider != "imap"
            || profile
                .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
                .is_some_and(|p| p["status"] == "demo")
        {
            return Err(AppError::invalid("不能覆盖演示账号或其他协议账号"));
        }
        Ok(Some(generation))
    } else {
        Ok(None)
    }
}
pub(crate) fn persist(
    db: &SharedDb,
    mut account: RealAccount,
    expected: Option<String>,
    secret: String,
) -> Result<String, AppError> {
    let id = account
        .id
        .clone()
        .unwrap_or_else(|| format!("imap:{}", uuid::Uuid::new_v4()));
    account.id = Some(id.clone());
    credentials::with_account("imap", &id, || {
        {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            let current: Option<String> = conn
                .query_row(
                    "SELECT sync_generation FROM accounts WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| AppError::storage("无法核对账号版本"))?;
            if current != expected {
                return Err(AppError::conflict("授权期间账号已改变，旧结果未保存"));
            }
        }
        let old = if credentials::exists("imap", &id)? {
            Some(credentials::secret_for("imap", &id)?)
        } else {
            None
        };
        credentials::set("imap", &id, &secret)?;
        let saved = db
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))
            .and_then(|mut conn| {
                mail_store::save_real_account(&mut conn, &account, expected.as_deref())
            });
        if let Err(error) = saved {
            let restored = if let Some(old) = old {
                credentials::set("imap", &id, &old)
            } else {
                credentials::delete("imap", &id)
            };
            if restored.is_err() {
                let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
                crate::imap_jobs::pause(&conn, &id)?;
                return Err(AppError::credentials(
                    "账号未保存且凭据恢复失败，请重新授权后再同步",
                ));
            }
            return Err(error);
        }
        crate::mail_sync::forget_session(&id);
        crate::mail_oauth::forget_token(&id);
        Ok(id.clone())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn failed_account_commit_restores_keyring_and_stale_authorization_cannot_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = crate::storage::initialize(&dir.path().join("mail.db")).unwrap();
        let id = format!("selftest-{}", uuid::Uuid::new_v4());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = credentials::delete("imap", &self.0);
            }
        }
        let _cleanup = Cleanup(id.clone());
        let account = RealAccount {
            id: Some(id.clone()),
            name: "Account".into(),
            email: "test@example.test".into(),
            imap_host: "imap.example.test".into(),
            imap_port: 993,
            imap_security: Default::default(),
            smtp_host: "".into(),
            smtp_port: 465,
            auth_user: "test@example.test".into(),
            avatar: "T".into(),
            color: "teal".into(),
            receive_only: true,
            oauth: None,
        };
        mail_store::save_real_account(&mut conn, &account, None).unwrap();
        conn.execute("INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,received_at) VALUES ('cached',?1,'imap:inbox:10:1','cached','Old mail','Sender','sender@example.test','2026-11-11T00:00:00Z')",[&id]).unwrap();
        let generation = existing_generation(&conn, &account).unwrap();
        let configuration = configuration(&conn, &id).unwrap();
        assert_eq!(configuration.auth_user, account.auth_user);
        credentials::set("imap", &id, "synthetic-old").unwrap();
        conn.execute_batch("CREATE TRIGGER fail_account BEFORE UPDATE ON accounts BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        let db = Arc::new(Mutex::new(conn));
        assert!(persist(
            &db,
            account.clone(),
            generation.clone(),
            "synthetic-new".into()
        )
        .is_err());
        assert_eq!(
            credentials::secret_for("imap", &id).unwrap(),
            "synthetic-old"
        );
        assert!(
            !mail_store::message_by_id(&db.lock().unwrap(), &id, "cached")
                .unwrap()
                .remote_missing
        );

        db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_account;")
            .unwrap();
        persist(
            &db,
            account.clone(),
            generation.clone(),
            "synthetic-new".into(),
        )
        .unwrap();
        assert!(
            mail_store::message_by_id(&db.lock().unwrap(), &id, "cached")
                .unwrap()
                .remote_missing
        );
        assert_ne!(
            existing_generation(&db.lock().unwrap(), &account).unwrap(),
            generation
        );
        assert_eq!(
            persist(&db, account, generation, "synthetic-stale".into())
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(
            credentials::secret_for("imap", &id).unwrap(),
            "synthetic-new"
        );
    }
}
