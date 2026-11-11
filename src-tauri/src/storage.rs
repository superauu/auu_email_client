use rusqlite::Connection;
use std::path::Path;

const LATEST_VERSION: i64 = 9;
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_foundation.sql")),
    (2, include_str!("../migrations/002_mail_store.sql")),
    (3, include_str!("../migrations/003_local_drafts.sql")),
    (4, include_str!("../migrations/004_sync_generation.sql")),
    (5, include_str!("../migrations/005_message_changes.sql")),
    (6, include_str!("../migrations/006_remote_recovery.sql")),
    (7, include_str!("../migrations/007_recovery_scan.sql")),
    (
        8,
        include_str!("../migrations/008_fingerprint_provenance.sql"),
    ),
    (9, include_str!("../migrations/009_imap_completion.sql")),
];

pub fn initialize(path: &Path) -> Result<Connection, String> {
    let mut connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    migrate(&mut connection)?;
    crate::message_changes::recover(&mut connection).map_err(|error| error.message)?;
    crate::mail_recovery::recover(&mut connection).map_err(|error| error.message)?;
    crate::imap_jobs::recover(&connection).map_err(|error| error.message)?;
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
    fn v8_upgrade_keeps_cached_mail_and_edits_without_inventing_raw_or_auto_sync() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (_, script) in MIGRATIONS.iter().filter(|(v, _)| *v <= 8) {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", 8).unwrap();
        crate::message_changes::tests::seed(&conn);
        conn.execute_batch("UPDATE messages SET starred=1,state_revision=1 WHERE id='m'; INSERT INTO message_changes(account_id,message_id,field,value,revision,state,generation,source_folder,source_validity,source_uid,updated_at) SELECT 'acc','m','starred',1,1,'queued',sync_generation,'INBOX',10,1,'2026-11-11T00:00:00Z' FROM accounts WHERE id='acc';").unwrap();
        migrate(&mut conn).unwrap();
        let m = crate::mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(m.starred);
        assert_eq!(m.changes.len(), 1);
        let key: Option<String> = conn
            .query_row("SELECT raw_cache_key FROM messages WHERE id='m'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(key.is_none());
        let settings = crate::imap_settings::read(&conn, "acc").unwrap();
        assert!(settings.sync_all_folders);
        assert!(!settings.history_enabled);
        assert_eq!(settings.auto_sync_seconds, 0);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM imap_sync_requests", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(conn
            .execute(
                "INSERT INTO imap_body_requests VALUES ('other','m','generation',1,'queued',NULL)",
                []
            )
            .is_err());
    }

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
