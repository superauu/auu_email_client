use rusqlite::Connection;
use std::path::Path;

const LATEST_VERSION: i64 = 2;
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_foundation.sql")),
    (2, include_str!("../migrations/002_mail_store.sql")),
];

pub fn initialize(path: &Path) -> Result<Connection, String> {
    let mut connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    migrate(&mut connection)?;
    Ok(connection)
}

pub fn migrate(connection: &mut Connection) -> Result<(), String> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    let current: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if current > LATEST_VERSION {
        return Err("数据库由更新版本创建，请使用对应版本打开。".into());
    }
    for &(version, script) in MIGRATIONS.iter().filter(|&&(version, _)| version > current) {
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute_batch(script)
            .map_err(|e| format!("迁移 v{version} 失败: {e}"))?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_repeatable_and_rejects_newer_databases() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db).unwrap();
        migrate(&mut db).unwrap();
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
        let columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('messages')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|c| c == "recipients_json"));
        let account_columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('accounts')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(account_columns.iter().any(|c| c == "profile_json"));
        db.pragma_update(None, "user_version", 99).unwrap();
        assert!(migrate(&mut db).is_err());
    }

    #[test]
    fn fresh_database_reaches_latest_version_in_one_pass() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db).unwrap();
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    #[test]
    fn folder_membership_cannot_cross_accounts() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db).unwrap();
        db.execute_batch("INSERT INTO accounts (id, provider, email, display_name, created_at) VALUES ('a','imap','a@example.test','A','2026-01-01T00:00:00Z');
          INSERT INTO accounts (id, provider, email, display_name, created_at) VALUES ('b','imap','b@example.test','B','2026-01-01T00:00:00Z');
          INSERT INTO folders VALUES ('f','b','remote-f','Inbox','inbox');
          INSERT INTO messages (id,account_id,remote_id,thread_id,subject,sender_name,sender_email,received_at) VALUES ('m','a','remote-m','t','Hi','A','a@example.test','2026-01-01T00:00:00Z');").unwrap();
        assert!(db
            .execute("INSERT INTO message_folders VALUES ('a','m','f')", [])
            .is_err());
    }

    #[test]
    fn deleting_message_updates_search_and_preserves_local_event() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db).unwrap();
        db.execute_batch("INSERT INTO accounts (id, provider, email, display_name, created_at) VALUES ('a','imap','a@example.test','A','2026-01-01T00:00:00Z');
          INSERT INTO messages (id,account_id,remote_id,thread_id,subject,sender_name,sender_email,received_at) VALUES ('m','a','remote-m','t','Review','A','a@example.test','2026-01-01T00:00:00Z');
          INSERT INTO mail_events VALUES ('e','a','m','Review','2026-01-02T00:00:00Z','UTC',0,'local');").unwrap();
        let found: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'Review'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);
        db.execute("DELETE FROM messages WHERE id = 'm'", [])
            .unwrap();
        let link: Option<String> = db
            .query_row(
                "SELECT message_id FROM mail_events WHERE id = 'e'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(link, None);
        let remaining: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'Review'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
