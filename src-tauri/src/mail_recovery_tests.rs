use super::*;
use crate::{
    imap_writeback::tests::{session, step, Hook},
    message_changes,
};
use std::sync::{Arc, Mutex};
#[path = "mail_recovery_extended_tests.rs"]
mod extended;
#[path = "mail_recovery_scan_tests.rs"]
mod paged;
#[path = "mail_recovery_wide_tests.rs"]
mod wide;
fn body() -> String {
    "Subject: Test\r\nFrom: Sender <sender@example.test>\r\nContent-Type: text/plain\r\n\r\nBody"
        .into()
}
fn examine_step(folder: &str, validity: u32) -> (String, String) {
    step(&format!("EXAMINE \"{folder}\""),&format!("* 1 EXISTS\r\n* OK [UIDVALIDITY {validity}] valid\r\n{{tag}} OK [READ-ONLY] examined\r\n"))
}
fn mail(uid: u32, raw: &str) -> Vec<(String, String)> {
    vec![step(&format!("UID FETCH {uid} (UID RFC822.SIZE)"),&format!("* 1 FETCH (UID {uid} RFC822.SIZE {})\r\n{{tag}} OK size\r\n",raw.len())),step(&format!("UID FETCH {uid} (UID INTERNALDATE BODY.PEEK[])"),&format!("* 1 FETCH (UID {uid} INTERNALDATE \"11-Nov-2026 00:00:00 +0000\" BODY[] {{{}}}\r\n{raw})\r\n{{tag}} OK body\r\n",raw.len()))]
}
fn absent(uid: u32) -> (String, String) {
    step(&format!("UID FETCH {uid}"), "{tag} OK absent\r\n")
}
fn db(legacy: bool, unknown: bool) -> SharedDb {
    let mut conn = message_changes::tests::fixture();
    if legacy {
        conn.execute("UPDATE messages SET remote_folder=NULL,remote_uid=NULL,remote_uid_validity=NULL,remote_generation=NULL",[]).unwrap();
    } else {
        conn.execute(
            "UPDATE messages SET content_hash=?1,content_hash_verified=1",
            [hash(body().as_bytes())],
        )
        .unwrap();
    }
    message_changes::update(
        &mut conn,
        "acc",
        "m",
        &message_changes::Patch {
            starred: Some(true),
            archive: unknown.then_some(true),
            ..Default::default()
        },
    )
    .unwrap();
    if unknown {
        conn.execute("UPDATE message_changes SET state='unknown',target_folder='Archive' WHERE field='archive'",[]).unwrap();
    }
    Arc::new(Mutex::new(conn))
}
fn request_action(db: &SharedDb, action: Action) {
    let mut conn = db.lock().unwrap();
    let rev = mail_store::message_by_id(&conn, "acc", "m")
        .unwrap()
        .state_revision;
    request(&mut conn, "acc", "m", rev, action).unwrap();
}
fn state(db: &SharedDb) -> String {
    state_string(&db.lock().unwrap())
}
fn state_string(conn: &Connection) -> String {
    status(conn, "m").unwrap().unwrap().state
}
fn run(db: &SharedDb, steps: Vec<(String, String)>, hook: Option<Hook>) -> Vec<String> {
    let (mut session, _, log) = session(steps, hook);
    process(db, "acc", &mut session, &["INBOX".into(), "Archive".into()]).unwrap();
    let result = log.lock().unwrap().clone();
    assert!(!result
        .iter()
        .any(|s| s.contains("STORE") || s.contains("MOVE") || s.contains("EXPUNGE")));
    result
}
fn legacy_steps() -> Vec<(String, String)> {
    let mut steps = vec![examine_step("INBOX", 10)];
    steps.extend(mail(1, &body()));
    steps
}
fn archive_steps(source_present: bool, matches: usize) -> Vec<(String, String)> {
    let mut steps = vec![examine_step("INBOX", 10)];
    if source_present {
        steps.extend(mail(1, &body()));
    } else {
        steps.push(absent(1));
    }
    steps.push(examine_step("Archive", 20));
    let ids = (0..matches)
        .map(|n| (7 + n).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    steps.push(step(
        "UID SEARCH ALL",
        &format!(
            "* SEARCH{}\r\n{{tag}} OK search\r\n",
            if ids.is_empty() {
                String::new()
            } else {
                format!(" {ids}")
            }
        ),
    ));
    for n in 0..matches {
        steps.extend(mail((7 + n) as u32, &body()));
    }
    steps.push(examine_step("INBOX", 10));
    if source_present {
        steps.extend(mail(1, &body()));
    } else {
        steps.push(absent(1));
    }
    steps
}
#[test]
fn legacy_verifies_content_then_revalidates_on_explicit_confirm_and_requeues_intents() {
    let db = db(true, false);
    request_action(&db, Action::Check);
    run(&db, legacy_steps(), None);
    assert_eq!(state(&db), "located");
    assert_eq!(
        status(&db.lock().unwrap(), "m")
            .unwrap()
            .unwrap()
            .evidence
            .as_deref(),
        Some("cached-fields")
    );
    assert!(message_changes::pending(&db.lock().unwrap(), "acc")
        .unwrap()
        .is_empty());
    request_action(&db, Action::Confirm);
    run(&db, legacy_steps(), None);
    assert_eq!(state(&db), "restored");
    let conn = db.lock().unwrap();
    let saved = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert!(saved.starred);
    assert_eq!(saved.body_text, "Body");
    assert!(saved.content_hash.is_some());
    let intents = message_changes::pending(&conn, "acc").unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].folder, "INBOX");
    conn.execute("UPDATE accounts SET email='changed@example.test'", [])
        .unwrap();
    assert_eq!(state_string(&conn), "available");
}
#[test]
fn legacy_missing_uid_changed_validity_content_and_ambiguous_folder_do_not_bind() {
    for scenario in 0..4 {
        let db = db(true, false);
        if scenario == 3 {
            db.lock()
                .unwrap()
                .execute(
                    "INSERT INTO folders VALUES ('in2','acc','OtherInbox','OtherInbox','inbox')",
                    [],
                )
                .unwrap();
        }
        request_action(&db, Action::Check);
        let steps = match scenario {
            0 => vec![examine_step("INBOX", 10), absent(1)],
            1 => vec![examine_step("INBOX", 99)],
            2 => {
                let mut steps = vec![examine_step("INBOX", 10)];
                steps.extend(mail(1, &body().replace("Body", "Different")));
                steps
            }
            _ => vec![],
        };
        run(&db, steps, None);
        assert_eq!(state(&db), "unresolved");
        let conn = db.lock().unwrap();
        let row = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(row.content_hash.is_none());
        assert!(row.starred);
        assert!(message_changes::pending(&conn, "acc").unwrap().is_empty());
    }
}
#[test]
fn confirmed_unknown_move_merges_unedited_target_cache_and_preserves_references() {
    let db = db(false, true);
    {
        let conn = db.lock().unwrap();
        conn.execute_batch("INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,body_text,received_at) VALUES ('duplicate','acc','imap:archive:20:7','duplicate','Test','Sender','sender@example.test','Body','2026-11-11T00:00:00Z');
         INSERT INTO labels VALUES ('tag',NULL,NULL,'Tag','teal','local');INSERT INTO message_labels VALUES ('duplicate','tag');
         INSERT INTO mail_events VALUES ('event','acc','duplicate','Event','2026-11-11T00:00:00Z','UTC',0,'local');
         INSERT INTO drafts(id,account_id,reply_to_id,updated_at) VALUES ('draft','acc','duplicate','2026-11-11T00:00:00Z');").unwrap();
    }
    request_action(&db, Action::Check);
    run(&db, archive_steps(false, 1), None);
    assert_eq!(state(&db), "located");
    assert!(message_changes::statuses(&db.lock().unwrap(), "m")
        .unwrap()
        .iter()
        .any(|c| c.state == "unknown"));
    request_action(&db, Action::Confirm);
    run(&db, archive_steps(false, 1), None);
    assert_eq!(state(&db), "restored");
    let conn = db.lock().unwrap();
    let saved = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert_eq!(saved.remote_id.as_deref(), Some("imap:archive:20:7"));
    assert_eq!(saved.mailbox, "archive");
    assert!(saved.starred);
    assert_eq!(saved.label_ids, vec!["tag"]);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    for sql in [
        "SELECT message_id FROM mail_events",
        "SELECT reply_to_id FROM drafts",
    ] {
        assert_eq!(
            conn.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap(),
            "m"
        );
    }
    let pending = message_changes::pending(&conn, "acc").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].uid, 7);
}
#[test]
fn unknown_move_can_be_explicitly_requeued_only_after_source_present_check() {
    let db = db(false, true);
    request_action(&db, Action::Check);
    run(&db, archive_steps(true, 0), None);
    assert_eq!(state(&db), "source-present");
    {
        let mut conn = db.lock().unwrap();
        let rev = mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .state_revision;
        assert!(request(&mut conn, "acc", "m", rev, Action::Confirm).is_err());
    }
    request_action(&db, Action::RetryArchive);
    run(&db, archive_steps(true, 0), None);
    let conn = db.lock().unwrap();
    assert_eq!(state_string(&conn), "restored");
    assert!(message_changes::pending(&conn, "acc")
        .unwrap()
        .iter()
        .any(|intent| intent.field == "archive"));
}
#[test]
fn both_copies_duplicate_matches_and_missing_matches_remain_unknown() {
    for (present, count) in [(true, 1), (false, 2), (false, 0)] {
        let db = db(false, true);
        request_action(&db, Action::Check);
        run(&db, archive_steps(present, count), None);
        assert_eq!(state(&db), "unresolved");
        assert!(message_changes::statuses(&db.lock().unwrap(), "m")
            .unwrap()
            .iter()
            .any(|c| c.field == "archive" && c.state == "unknown"));
    }
}
#[test]
fn edits_or_account_change_during_unlocked_io_invalidate_evidence() {
    for account_change in [false, true] {
        let db = db(true, false);
        request_action(&db, Action::Check);
        let other = db.clone();
        let mut once = false;
        let hook: Hook = Box::new(move |command| {
            if !once && command.contains("BODY.PEEK") {
                once = true;
                let mut conn = other.try_lock().expect("network must not hold DB guard");
                if account_change {
                    conn.execute("UPDATE accounts SET email='changed@example.test'", [])
                        .unwrap();
                } else {
                    message_changes::update(
                        &mut conn,
                        "acc",
                        "m",
                        &message_changes::Patch {
                            starred: Some(false),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
            }
        });
        run(&db, legacy_steps(), Some(hook));
        assert_eq!(state(&db), "unresolved");
        assert!(mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .content_hash
            .is_none());
    }
}
#[test]
fn stale_review_and_changed_candidate_cannot_be_confirmed() {
    let db = db(true, false);
    request_action(&db, Action::Check);
    run(&db, legacy_steps(), None);
    {
        let mut conn = db.lock().unwrap();
        let before = mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .state_revision;
        message_changes::update(
            &mut conn,
            "acc",
            "m",
            &message_changes::Patch {
                starred: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(state_string(&conn), "stale");
        assert!(request(&mut conn, "acc", "m", before, Action::Confirm).is_err());
    }
    request_action(&db, Action::Check);
    run(&db, legacy_steps(), None);
    request_action(&db, Action::Confirm);
    let mut steps = vec![examine_step("INBOX", 10)];
    steps.extend(mail(1, &body().replace("Body", "Changed")));
    run(&db, steps, None);
    assert_eq!(state(&db), "unresolved");
}
#[test]
fn interruption_is_persistent_and_recovery_does_not_resume_old_confirmation() {
    let db = db(true, false);
    request_action(&db, Action::Check);
    let (mut s, _, _) = session(vec![examine_step("INBOX", 10), step("UID FETCH", "")], None);
    assert!(process(&db, "acc", &mut s, &["INBOX".into()]).is_err());
    assert_eq!(state(&db), "failed");
    request_action(&db, Action::Check);
    {
        let mut conn = db.lock().unwrap();
        begin(&mut conn, "acc", "m").unwrap();
        recover(&mut conn).unwrap();
        assert_eq!(state_string(&conn), "queued");
    }
    run(&db, legacy_steps(), None);
    assert_eq!(state(&db), "located");
}
#[test]
fn merge_failure_rolls_back_and_keeps_unknown_intent() {
    let db = db(false, true);
    request_action(&db, Action::Check);
    run(&db, archive_steps(false, 1), None);
    request_action(&db, Action::Confirm);
    {
        let conn = db.lock().unwrap();
        conn.execute_batch("CREATE TRIGGER fail_rebind BEFORE UPDATE OF remote_id ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    }
    let (mut s, _, _) = session(archive_steps(false, 1), None);
    assert!(process(&db, "acc", &mut s, &["INBOX".into(), "Archive".into()]).is_err());
    assert_eq!(state(&db), "unresolved");
    let conn = db.lock().unwrap();
    assert_eq!(
        mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .remote_id
            .as_deref(),
        Some("imap:inbox:10:1")
    );
    assert!(message_changes::statuses(&conn, "m")
        .unwrap()
        .iter()
        .any(|c| c.state == "unknown"));
}

#[test]
fn queued_and_review_requests_are_durable_account_scoped_and_can_be_cancelled_locally() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.sqlite");
    {
        let mut conn = crate::storage::initialize(&path).unwrap();
        message_changes::tests::seed(&conn);
        conn.execute("UPDATE messages SET remote_generation=NULL", [])
            .unwrap();
        assert!(request(&mut conn, "wrong-account", "m", 0, Action::Check).is_err());
        request(&mut conn, "acc", "m", 0, Action::Check).unwrap();
    }
    let db = Arc::new(Mutex::new(crate::storage::initialize(&path).unwrap()));
    assert_eq!(state(&db), "queued");
    run(&db, legacy_steps(), None);
    assert_eq!(state(&db), "located");
    drop(db);
    let mut conn = crate::storage::initialize(&path).unwrap();
    assert_eq!(state_string(&conn), "located");
    message_changes::keep_local(&mut conn, "acc", "m").unwrap();
    assert_eq!(state_string(&conn), "local");
}

#[test]
fn recovery_request_between_flag_snapshot_and_dispatch_pauses_the_write() {
    let db = db(false, false);
    let other = db.clone();
    let mut once = false;
    let hook: Hook = Box::new(move |command| {
        if !once && command.contains("UID FETCH") {
            once = true;
            let mut conn = other.try_lock().unwrap();
            let rev = mail_store::message_by_id(&conn, "acc", "m")
                .unwrap()
                .state_revision;
            // This test explicitly requests a check on an already verified row with a recoverable conflict.
            conn.execute("UPDATE messages SET remote_generation=NULL", [])
                .unwrap();
            request(&mut conn, "acc", "m", rev, Action::Check).unwrap();
        }
    });
    let (mut s, t, log) = session(
        vec![
            step(
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK capabilities\r\n",
            ),
            step(
                "SELECT",
                "* OK [UIDVALIDITY 10] valid\r\n{tag} OK selected\r\n",
            ),
            step(
                "UID FETCH 1",
                "* 1 FETCH (UID 1 FLAGS ())\r\n{tag} OK flags\r\n",
            ),
        ],
        Some(hook),
    );
    crate::imap_writeback::flush(&db, "acc", &mut s, Some("Archive"), Some(&t)).unwrap();
    assert!(!log.lock().unwrap().iter().any(|c| c.contains("STORE")));
    assert_eq!(state(&db), "queued");
}

#[test]
fn edited_target_duplicate_is_preserved_when_confirming_archive() {
    let db = db(false, true);
    request_action(&db, Action::Check);
    run(&db, archive_steps(false, 1), None);
    request_action(&db, Action::Confirm);
    {
        let mut conn = db.lock().unwrap();
        conn.execute_batch("INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,body_text,received_at) VALUES ('duplicate','acc','imap:archive:20:7','duplicate','Test','Sender','sender@example.test','Body','2026-11-11T00:00:00Z');").unwrap();
        message_changes::update(
            &mut conn,
            "acc",
            "duplicate",
            &message_changes::Patch {
                unread: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let (mut s, _, _) = session(archive_steps(false, 1), None);
    assert!(process(&db, "acc", &mut s, &["INBOX".into(), "Archive".into()]).is_err());
    let conn = db.lock().unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(state_string(&conn), "unresolved");
    assert_eq!(
        message_changes::statuses(&conn, "duplicate").unwrap().len(),
        1
    );
}

#[test]
fn keep_local_during_network_check_cancels_the_pending_result() {
    let db = db(true, false);
    request_action(&db, Action::Check);
    let other = db.clone();
    let mut once = false;
    let hook: Hook = Box::new(move |command| {
        if !once && command.contains("BODY.PEEK") {
            once = true;
            message_changes::keep_local(&mut other.try_lock().unwrap(), "acc", "m").unwrap();
        }
    });
    run(&db, legacy_steps(), Some(hook));
    assert_eq!(state(&db), "local");
    let conn = db.lock().unwrap();
    assert!(mail_store::message_by_id(&conn, "acc", "m")
        .unwrap()
        .content_hash
        .is_none());
    assert!(message_changes::statuses(&conn, "m")
        .unwrap()
        .iter()
        .all(|c| c.state == "local"));
}

#[test]
fn invalid_size_and_wrong_uid_responses_do_not_recover_a_locator() {
    for response in [
        "* 1 FETCH (UID 1 RFC822.SIZE 1048577)\r\n{tag} OK size\r\n",
        "* 1 FETCH (UID 2 RFC822.SIZE 80)\r\n{tag} OK size\r\n",
        "* 1 FETCH (UID 1)\r\n{tag} OK size\r\n",
    ] {
        let db = db(true, false);
        request_action(&db, Action::Check);
        run(
            &db,
            vec![examine_step("INBOX", 10), step("UID FETCH", response)],
            None,
        );
        assert_eq!(state(&db), "unresolved");
        assert!(mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .content_hash
            .is_none());
    }
}
