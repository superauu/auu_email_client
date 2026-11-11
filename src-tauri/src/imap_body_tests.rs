use super::*;
use crate::{
    imap_writeback::tests::{session, step, Hook},
    message_changes,
};
use std::sync::{Arc, Mutex};
fn raw() -> &'static str {
    "Subject: Test\r\nFrom: Sender <sender@example.test>\r\nContent-Type: text/plain\r\n\r\nBody"
}
fn fixture() -> (tempfile::TempDir, SharedDb) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = crate::storage::initialize(&dir.path().join("mail.db")).unwrap();
    message_changes::tests::seed(&conn);
    message_changes::update(
        &mut conn,
        "acc",
        "m",
        &message_changes::Patch {
            starred: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let revision = mail_store::message_by_id(&conn, "acc", "m")
        .unwrap()
        .state_revision;
    request(&mut conn, "acc", "m", revision).unwrap();
    (dir, Arc::new(Mutex::new(conn)))
}
fn boundary() -> (String, String) {
    step("EXAMINE", "* 1 EXISTS\r\n* OK [UIDVALIDITY 10] valid\r\n* OK [UIDNEXT 2] next\r\n{tag} OK examined\r\n")
}
fn steps() -> Vec<(String, String)> {
    vec![boundary(),step("RFC822.SIZE",&format!("* 1 FETCH (UID 1 RFC822.SIZE {})\r\n{{tag}} OK size\r\n",raw().len())),step("BODY.PEEK[]",&format!("* 1 FETCH (UID 1 FLAGS () INTERNALDATE \"11-Nov-2026 00:00:00 +0000\" BODY[] {{{}}}\r\n{})\r\n{{tag}} OK body\r\n",raw().len(),raw())),boundary()]
}
#[test]
fn raw_re_download_preserves_intents_and_does_not_upgrade_weak_evidence() {
    let (_dir, db) = fixture();
    let revision = message_changes::pending(&db.lock().unwrap(), "acc").unwrap()[0].revision;
    let (mut s, _, _) = session(steps(), None);
    process(&db, "acc", &mut s).unwrap();
    let conn = db.lock().unwrap();
    let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert!(m.starred && !m.content_hash_verified && m.locator_verified);
    assert!(m.body_download.is_none());
    assert!(crate::mail_attachments::load(&conn, "acc", "m", 0).is_err());
    let root = crate::mail_cache::root(&conn).unwrap();
    let key = m.content_hash.unwrap();
    assert_eq!(
        crate::mail_cache::read(&root, &key).unwrap(),
        raw().as_bytes()
    );
    assert_eq!(
        message_changes::pending(&conn, "acc").unwrap()[0].revision,
        revision
    );
}
#[test]
fn an_inflight_edit_or_newer_download_request_is_not_overwritten() {
    for newer in [false, true] {
        let (_dir, db) = fixture();
        let other = db.clone();
        let hook: Hook = Box::new(move |cmd| {
            if cmd.contains("BODY.PEEK[]") {
                let mut conn = other.try_lock().unwrap();
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
                if newer {
                    let revision = mail_store::message_by_id(&conn, "acc", "m")
                        .unwrap()
                        .state_revision;
                    request(&mut conn, "acc", "m", revision).unwrap();
                }
            }
        });
        let (mut s, _, _) = session(steps(), Some(hook));
        process(&db, "acc", &mut s).unwrap();
        let conn = db.lock().unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!m.starred);
        assert_eq!(
            m.body_download.unwrap().state,
            if newer { "queued" } else { "failed" }
        );
    }
}
#[test]
fn a_network_failure_keeps_a_durable_request_and_original_cache() {
    let (dir, db) = fixture();
    let (mut s, _, _) = session(vec![boundary(), step("RFC822.SIZE", "")], None);
    assert!(process(&db, "acc", &mut s).is_err());
    drop(db);
    let conn = crate::storage::initialize(&dir.path().join("mail.db")).unwrap();
    let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert!(m.starred);
    assert_eq!(m.body_download.unwrap().state, "queued");
    assert_eq!(m.body_text, "Body");
    assert!(!m.content_hash_verified);
}
