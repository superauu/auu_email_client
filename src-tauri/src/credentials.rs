//! OS credential service (Windows Credential Manager via keyring).
//! Entries are scoped per provider + account. Secrets are write-only for the rest of the app:
//! no command and no error message ever returns a secret or echoes the platform error verbatim.
use crate::error::AppError;
use keyring::Entry;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};
type AccountLocks = Mutex<HashMap<String, Weak<Mutex<()>>>>;
static ACCOUNT_LOCKS: OnceLock<AccountLocks> = OnceLock::new();
/// Serializes credential rotation with account reconfiguration. Never call with a DB guard.
pub(crate) fn with_account<T>(
    provider: &str,
    id: &str,
    work: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let _ = entry(provider, id)?;
    let key = format!("{provider}:{id}");
    let lock = {
        let mut locks = ACCOUNT_LOCKS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| AppError::credentials("凭据锁不可用"))?;
        locks.retain(|_, v| v.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        }
    };
    let _guard = lock
        .lock()
        .map_err(|_| AppError::credentials("账号凭据不可用"))?;
    work()
}

const SERVICE: &str = "AuuMail";
const PROVIDERS: [&str; 4] = ["imap", "pop3", "microsoft", "gmail"];
const MAX_USER_LEN: usize = 200;

fn entry(provider: &str, account_id: &str) -> Result<Entry, AppError> {
    if !PROVIDERS.contains(&provider) {
        return Err(AppError::invalid(format!("不支持的账号协议: {provider}")));
    }
    let user = format!("{provider}:{account_id}");
    if account_id.is_empty() || user.len() > MAX_USER_LEN || user.chars().any(|c| c.is_control()) {
        return Err(AppError::invalid("凭据标识超出允许范围"));
    }
    Entry::new(SERVICE, &user).map_err(|_| AppError::credentials("系统凭据服务不可用"))
}

/// Only presence is exposed; the secret itself never crosses the IPC boundary back to the UI.
pub fn exists(provider: &str, account_id: &str) -> Result<bool, AppError> {
    match entry(provider, account_id)?.get_password() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(_) => Err(AppError::credentials("凭据状态检查失败")),
    }
}

pub fn set(provider: &str, account_id: &str, secret: &str) -> Result<(), AppError> {
    if secret.is_empty() {
        return Err(AppError::invalid("凭据内容不能为空"));
    }
    entry(provider, account_id)?
        .set_password(secret)
        .map_err(|_| AppError::credentials("凭据保存失败"))
}

pub fn delete(provider: &str, account_id: &str) -> Result<(), AppError> {
    match entry(provider, account_id)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // deleting a missing credential stays idempotent
        Err(_) => Err(AppError::credentials("凭据删除失败")),
    }
}

/// Internal read for sync tasks. Never exposed as an IPC command.
pub fn secret_for(provider: &str, account_id: &str) -> Result<String, AppError> {
    match entry(provider, account_id)?.get_password() {
        Ok(secret) => Ok(secret),
        Err(keyring::Error::NoEntry) => Err(AppError::credentials(
            "系统凭据库中没有该账号的密码，请重新添加账号",
        )),
        Err(_) => Err(AppError::credentials("无法读取系统凭据")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_roundtrip_is_scoped_and_cleaned_up() {
        let account_id = format!(
            "selftest-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        // A stale entry from an interrupted run must not fail the test.
        let _ = delete("imap", &account_id);
        assert!(!exists("imap", &account_id).unwrap());

        set("imap", &account_id, "secret-value").unwrap();
        assert!(exists("imap", &account_id).unwrap());
        let stored = Entry::new(SERVICE, &format!("imap:{account_id}"))
            .unwrap()
            .get_password()
            .unwrap();
        assert_eq!(stored, "secret-value");

        delete("imap", &account_id).unwrap();
        assert!(!exists("imap", &account_id).unwrap());
        delete("imap", &account_id).unwrap(); // idempotent
    }

    #[test]
    fn unknown_provider_and_empty_secret_are_rejected_without_touching_the_store() {
        assert_eq!(exists("ftp", "selftest-x").unwrap_err().code, "invalid");
        assert_eq!(set("imap", "selftest-x", "").unwrap_err().code, "invalid");
    }
}
