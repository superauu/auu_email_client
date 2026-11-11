use super::tests::{db, fetch, first, fixture_fetch, plan};
use super::*;
use crate::imap_writeback::tests::{session, step};
fn condstore_boundary(seq: u64, read_only: bool) -> (String, String) {
    step(&format!("{} \"INBOX\" (CONDSTORE)",if read_only{"EXAMINE"}else{"SELECT"}),&format!("* 1 EXISTS\r\n* OK [UIDVALIDITY 1000] valid\r\n* OK [UIDNEXT 1002] next\r\n* OK [HIGHESTMODSEQ {seq}] highest\r\n{{tag}} OK selected\r\n"))
}
#[test]
fn condstore_keeps_uid_presence_checks_and_does_not_advance_past_a_local_race() {
    for (changed, race) in [(false, false), (true, false), (true, true)] {
        let db = db();
        first(&db);
        {
            let conn = db.lock().unwrap();
            let generation: String = conn
                .query_row("SELECT sync_generation FROM accounts", [], |r| r.get(0))
                .unwrap();
            conn.execute(
                "INSERT INTO imap_modseq VALUES ('acc','INBOX',?1,1000,'5')",
                [generation],
            )
            .unwrap();
        }
        let mut plan = plan(&db);
        plan.condstore = true;
        let seq = if changed { 6 } else { 5 };
        let mut steps = vec![listed(), condstore_boundary(seq, false)];
        if changed {
            steps.push(step(
                "CHANGEDSINCE 5",
                "* 1 FETCH (UID 1001 FLAGS (\\Flagged) MODSEQ (6))\r\n{tag} OK delta\r\n",
            ));
        } else {
            steps.extend([
                step(
                    "UID SEARCH UID 1001",
                    "* SEARCH 1001\r\n{tag} OK search\r\n",
                ),
                selected("INBOX", 1000, 1002, true),
            ]);
        }
        steps.extend([
            condstore_boundary(seq, true),
            step("LOGOUT", "* BYE bye\r\n{tag} OK logout\r\n"),
        ]);
        let other = db.clone();
        let hook: crate::imap_writeback::tests::Hook = Box::new(move |cmd| {
            if race && cmd.contains("CHANGEDSINCE") {
                crate::message_changes::update(
                    &mut other.try_lock().unwrap(),
                    "acc",
                    "acc:imap:inbox:1000:1001",
                    &crate::message_changes::Patch {
                        starred: Some(false),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        });
        let (s, trace, _) = session(steps, Some(hook));
        sync_with_fetcher(&db, &plan, s, Some(&trace), fixture_fetch).unwrap();
        let conn = db.lock().unwrap();
        let m = crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001").unwrap();
        assert_eq!(m.starred, changed && !race);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM imap_modseq", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            if race { 0 } else { 1 }
        );
    }
}
fn listed() -> (String, String) {
    step("LIST", "* LIST () \"/\" INBOX\r\n{tag} OK list\r\n")
}
fn selected(name: &str, validity: u32, next: u32, read_only: bool) -> (String, String) {
    step(&format!("{} \"{name}\"",if read_only{"EXAMINE"}else{"SELECT"}),&format!("* 1 EXISTS\r\n* OK [UIDVALIDITY {validity}] valid\r\n* OK [UIDNEXT {next}] next\r\n{{tag}} OK selected\r\n"))
}
fn reply(uid: u32, subject: &str) -> String {
    fetch(uid, 0, subject).replace("a0 OK fetched", "{tag} OK fetched")
}
#[test]
fn all_folder_sync_scopes_identical_uids_and_commits_complete_cached_messages() {
    let db = db();
    let settings = crate::imap_settings::Settings {
        sync_all_folders: true,
        ..Default::default()
    };
    crate::imap_settings::save(&db.lock().unwrap(), "acc", &settings).unwrap();
    let mut plan = plan(&db);
    let cache = tempfile::tempdir().unwrap();
    plan.cache_root = cache.path().into();
    let mut steps=vec![step("LIST","* LIST () \"/\" INBOX\r\n* LIST () \"/\" \"Projects/A\"\r\n* LIST () \"/\" \"Projects/B\"\r\n{tag} OK list\r\n")];
    for name in ["INBOX", "Projects/A", "Projects/B"] {
        let response = reply(1, name);
        let size = response
            .split("BODY[] {")
            .nth(1)
            .unwrap()
            .split('}')
            .next()
            .unwrap();
        steps.extend([
            selected(name, 10, 2, false),
            step(
                "RFC822.SIZE",
                &format!("* 1 FETCH (UID 1 RFC822.SIZE {size})\r\n{{tag}} OK sizes\r\n"),
            ),
            step("UID FETCH 1 (UID FLAGS", &response),
            selected(name, 10, 2, true),
        ]);
    }
    steps.push(step("LOGOUT", "* BYE bye\r\n{tag} OK logout\r\n"));
    let (s, trace, _) = session(steps, None);
    assert_eq!(
        sync_with_session_trace(&db, &plan, s, Some(&trace))
            .unwrap()
            .fetched_messages,
        3
    );
    let conn = db.lock().unwrap();
    let page = crate::mail_store::page_messages(&conn, &Default::default()).unwrap();
    assert_eq!(page.items.len(), 3);
    let ids: std::collections::HashSet<_> =
        page.items.iter().map(|m| m.remote_id.clone()).collect();
    assert_eq!(ids.len(), 3);
    let project = page
        .items
        .iter()
        .find(|m| m.subject == "Projects/B")
        .unwrap();
    assert_eq!(project.folder_ids, vec!["acc:folder:Projects/B"]);
    assert!(project.content_hash_verified);
    assert_eq!(
        crate::mail_store::page_messages(
            &conn,
            &crate::mail_store::MessageQuery {
                folder_id: Some("acc:folder:Projects/B".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .items
        .len(),
        1
    );
}
#[test]
fn historical_ranges_move_backwards_without_rewinding_the_incremental_cursor_or_local_flags() {
    let db = db();
    first(&db);
    crate::imap_settings::save(
        &db.lock().unwrap(),
        "acc",
        &crate::imap_settings::Settings {
            history_enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    let id = "acc:imap:inbox:1000:1001";
    crate::message_changes::update(
        &mut db.lock().unwrap(),
        "acc",
        id,
        &crate::message_changes::Patch {
            starred: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    db.lock()
        .unwrap()
        .execute("UPDATE message_changes SET state='conflict'", [])
        .unwrap();
    let (s, trace, _) = session(
        vec![
            listed(),
            selected("INBOX", 1000, 1002, false),
            step(
                "UID FETCH 1001 (UID FLAGS)",
                "* 1 FETCH (UID 1001 FLAGS (\\Seen))\r\n{tag} OK flags\r\n",
            ),
            step("UID FETCH 852:951", &reply(900, "History")),
            step("LOGOUT", "* BYE bye\r\n{tag} OK logout\r\n"),
        ],
        None,
    );
    let outcome = sync_with_fetcher(&db, &plan(&db), s, Some(&trace), fixture_fetch).unwrap();
    assert!(outcome.has_more);
    let conn = db.lock().unwrap();
    assert_eq!(
        read_cursor(&conn, "acc", "inbox")
            .unwrap()
            .unwrap()
            .last_uid,
        1001
    );
    assert_eq!(
        conn.query_row("SELECT before_uid FROM imap_history", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        852
    );
    assert!(
        crate::mail_store::message_by_id(&conn, "acc", id)
            .unwrap()
            .starred
    );
}
#[test]
fn expunge_requires_independent_uid_absence_and_preserves_edits_made_during_search() {
    for edit in [false, true] {
        let db = db();
        first(&db);
        let other = db.clone();
        let hook: crate::imap_writeback::tests::Hook = Box::new(move |command| {
            if command.contains("UID SEARCH") && edit {
                let mut conn = other.try_lock().unwrap();
                crate::message_changes::update(
                    &mut conn,
                    "acc",
                    "acc:imap:inbox:1000:1001",
                    &crate::message_changes::Patch {
                        starred: Some(true),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        });
        let (s, trace, _) = session(
            vec![
                listed(),
                selected("INBOX", 1000, 1002, false),
                step("UID FETCH 1001 (UID FLAGS)", "{tag} OK no flags\r\n"),
                step("UID SEARCH UID 1001", "* SEARCH\r\n{tag} OK absent\r\n"),
                selected("INBOX", 1000, 1002, true),
                step("LOGOUT", "* BYE bye\r\n{tag} OK logout\r\n"),
            ],
            Some(hook),
        );
        sync_with_fetcher(&db, &plan(&db), s, Some(&trace), fixture_fetch).unwrap();
        let conn = db.lock().unwrap();
        let m = crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001").unwrap();
        assert_eq!(m.remote_missing, !edit);
        assert_eq!(m.starred, edit);
        assert!(m.body_text.contains("Hello"));
    }
}
#[test]
fn reconfigured_account_uid_reuse_keeps_the_old_message_and_its_local_intent() {
    let db = db();
    first(&db);
    let id = "acc:imap:inbox:1000:1001";
    {
        let mut conn = db.lock().unwrap();
        crate::message_changes::update(
            &mut conn,
            "acc",
            id,
            &crate::message_changes::Patch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE accounts SET email='new@example.test' WHERE id='acc'",
            [],
        )
        .unwrap();
    }
    let plan = plan(&db);
    let body = ParsedMessage {
        uid: 1001,
        raw_cache_key: None,
        attachments: vec![],
        content_hash: "new-hash".into(),
        header_message_id: None,
        subject: "Different account mail".into(),
        sender_name: "Other".into(),
        sender_email: "other@example.test".into(),
        recipients_json: "[]".into(),
        body_text: "Unrelated content".into(),
        received_at: crate::now_utc(),
        unread: false,
        starred: false,
    };
    let old = plan.cursors["inbox"];
    commit_batch(&db, &plan, "inbox", "INBOX", old, old.unwrap(), &[body]).unwrap();
    let conn = db.lock().unwrap();
    let old = crate::mail_store::message_by_id(&conn, "acc", id).unwrap();
    assert!(old.starred && old.remote_missing && old.body_text.contains("Hello"));
    assert_eq!(old.remote_id.as_deref(), Some("imap:inbox:1000:1001"));
    assert_eq!(old.changes[0].state, "conflict");
    let page = crate::mail_store::page_messages(&conn, &Default::default()).unwrap();
    assert_eq!(page.items.len(), 2);
    let new = page
        .items
        .iter()
        .find(|m| m.subject == "Different account mail")
        .unwrap();
    assert_ne!(new.id, id);
    assert!(!new.starred);
    assert!(new.changes.is_empty());
}
#[test]
fn refused_condstore_falls_back_to_full_flags_without_a_watermark() {
    let db = db();
    first(&db);
    let mut plan = plan(&db);
    plan.condstore = true;
    let (s, trace, _) = session(
        vec![
            listed(),
            step("SELECT \"INBOX\" (CONDSTORE)", "{tag} BAD unsupported\r\n"),
            selected("INBOX", 1000, 1002, false),
            step(
                "UID FETCH 1001 (UID FLAGS)",
                "* 1 FETCH (UID 1001 FLAGS (\\Seen \\Flagged))\r\n{tag} OK flags\r\n",
            ),
            step("LOGOUT", "* BYE bye\r\n{tag} OK logout\r\n"),
        ],
        None,
    );
    sync_with_fetcher(&db, &plan, s, Some(&trace), fixture_fetch).unwrap();
    let conn = db.lock().unwrap();
    assert!(
        crate::mail_store::message_by_id(&conn, "acc", "acc:imap:inbox:1000:1001")
            .unwrap()
            .starred
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM imap_modseq", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
