use super::*;
fn search(ids: &[u32]) -> (String, String) {
    let list = ids
        .iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    step(
        "UID SEARCH ALL",
        &format!(
            "* SEARCH{}\r\n{{tag}} OK searched\r\n",
            if list.is_empty() {
                String::new()
            } else {
                format!(" {list}")
            }
        ),
    )
}
fn listing(folder: &str, validity: u32, ids: &[u32]) -> Vec<(String, String)> {
    vec![examine_step(folder, validity), search(ids)]
}
fn cross_steps(ids: &[(u32, bool)], offset: usize, initial: bool) -> Vec<(String, String)> {
    let uids = ids.iter().map(|(uid, _)| *uid).collect::<Vec<_>>();
    let mut steps = vec![];
    if initial {
        steps.push(examine_step("INBOX", 20));
        steps.push(examine_step("INBOX", 20));
    }
    steps.extend(listing("INBOX", 20, &uids));
    let page = &ids[offset..ids.len().min(offset + 8)];
    let mut matches = ids[..offset].iter().filter(|(_, m)| *m).count();
    if !page.is_empty() {
        steps.push(examine_step("INBOX", 20));
    }
    for (uid, matching) in page {
        steps.extend(mail(
            *uid,
            &if *matching {
                body()
            } else {
                body().replace("Body", "Different")
            },
        ));
        matches += usize::from(*matching);
        if matches > 1 {
            return steps;
        }
    }
    steps.extend(listing("INBOX", 20, &uids));
    if offset + page.len() == ids.len() && matches == 1 {
        let uid = ids.iter().find(|(_, matching)| *matching).unwrap().0;
        steps.push(examine_step("INBOX", 20));
        steps.extend(mail(uid, &body()));
    }
    steps
}
fn strong_cross() -> SharedDb {
    let db = db(false, false);
    db.lock()
        .unwrap()
        .execute("UPDATE message_changes SET state='conflict'", [])
        .unwrap();
    db
}
fn progress(db: &SharedDb) -> Progress {
    status(&db.lock().unwrap(), "m")
        .unwrap()
        .unwrap()
        .progress
        .unwrap()
}

#[test]
fn cross_uidvalidity_rebinding_scans_all_pages_and_rechecks_on_confirm() {
    let db = strong_cross();
    let ids = (1..10).map(|uid| (uid, uid == 9)).collect::<Vec<_>>();
    request_action(&db, Action::Check);
    run(&db, cross_steps(&ids, 0, true), None);
    assert_eq!(state(&db), "scanning");
    assert_eq!(progress(&db).checked, 8);
    {
        let mut conn = db.lock().unwrap();
        let rev = mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .state_revision;
        assert!(request(&mut conn, "acc", "m", rev, Action::Confirm).is_err());
    }
    request_action(&db, Action::Continue);
    run(&db, cross_steps(&ids, 8, false), None);
    assert_eq!(state(&db), "located");
    {
        let conn = db.lock().unwrap();
        let status = status(&conn, "m").unwrap().unwrap();
        assert!(status.cross_uid_validity);
        assert_eq!(status.location.unwrap().uid, 9);
        assert!(message_changes::pending(&conn, "acc").unwrap().is_empty());
    }
    request_action(&db, Action::Confirm);
    run(&db, cross_steps(&ids, 0, true), None);
    assert_eq!(state(&db), "scanning");
    request_action(&db, Action::Continue);
    run(&db, cross_steps(&ids, 8, false), None);
    assert_eq!(state(&db), "restored");
    let conn = db.lock().unwrap();
    let message = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert!(message.starred);
    assert_eq!(message.remote_id.as_deref(), Some("imap:inbox:20:9"));
    assert_eq!(message.body_text, "Body");
    let intents = message_changes::pending(&conn, "acc").unwrap();
    assert_eq!(intents[0].validity, 20);
    assert_eq!(intents[0].uid, 9);
}
#[test]
fn a_duplicate_on_a_later_page_never_becomes_a_unique_binding() {
    let db = strong_cross();
    let ids = (1..10)
        .map(|uid| (uid, uid == 1 || uid == 9))
        .collect::<Vec<_>>();
    request_action(&db, Action::Check);
    run(&db, cross_steps(&ids, 0, true), None);
    assert_eq!(state(&db), "scanning");
    assert_eq!(progress(&db).matches, 1);
    request_action(&db, Action::Continue);
    run(&db, cross_steps(&ids, 8, false), None);
    assert_eq!(state(&db), "unresolved");
    let conn = db.lock().unwrap();
    assert_eq!(
        mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .remote_id
            .as_deref(),
        Some("imap:inbox:10:1")
    );
}
#[test]
fn changed_uidvalidity_or_candidate_set_invalidates_saved_pages() {
    for validity in [20, 21] {
        let db = strong_cross();
        let ids = (1..10).map(|uid| (uid, uid == 9)).collect::<Vec<_>>();
        request_action(&db, Action::Check);
        run(&db, cross_steps(&ids, 0, true), None);
        request_action(&db, Action::Continue);
        run(
            &db,
            listing("INBOX", validity, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            None,
        );
        assert_eq!(state(&db), "unresolved");
        assert!(status(&db.lock().unwrap(), "m")
            .unwrap()
            .unwrap()
            .progress
            .is_none());
    }
}
#[test]
fn partial_scan_survives_reopen_and_can_resume_after_a_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages.sqlite");
    let mut conn = crate::storage::initialize(&path).unwrap();
    message_changes::tests::seed(&conn);
    conn.execute(
        "UPDATE messages SET content_hash=?1,content_hash_verified=1",
        [hash(body().as_bytes())],
    )
    .unwrap();
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
    let db = Arc::new(Mutex::new(conn));
    db.lock()
        .unwrap()
        .execute("UPDATE message_changes SET state='conflict'", [])
        .unwrap();
    let ids = (1..10).map(|uid| (uid, uid == 9)).collect::<Vec<_>>();
    request_action(&db, Action::Check);
    run(&db, cross_steps(&ids, 0, true), None);
    drop(db);
    let db = Arc::new(Mutex::new(crate::storage::initialize(&path).unwrap()));
    assert_eq!(state(&db), "scanning");
    request_action(&db, Action::Continue);
    let (mut s, _, _) = session(vec![step("EXAMINE", "")], None);
    assert!(process(&db, "acc", &mut s, &["INBOX".into()]).is_err());
    assert_eq!(state(&db), "failed");
    assert_eq!(progress(&db).checked, 8);
    request_action(&db, Action::Continue);
    run(&db, cross_steps(&ids, 8, false), None);
    assert_eq!(state(&db), "located");
}
#[test]
fn edits_between_pages_require_a_fresh_scan_and_keep_the_new_value() {
    let db = strong_cross();
    let ids = (1..10).map(|uid| (uid, uid == 9)).collect::<Vec<_>>();
    request_action(&db, Action::Check);
    run(&db, cross_steps(&ids, 0, true), None);
    let mut conn = db.lock().unwrap();
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
    let rev = mail_store::message_by_id(&conn, "acc", "m")
        .unwrap()
        .state_revision;
    assert!(request(&mut conn, "acc", "m", rev, Action::Continue).is_err());
    assert!(
        !mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .starred
    );
}
#[test]
fn edits_during_a_continuation_invalidate_the_last_page_without_holding_db_lock() {
    let db = strong_cross();
    let ids = (1..10).map(|uid| (uid, uid == 9)).collect::<Vec<_>>();
    request_action(&db, Action::Check);
    run(&db, cross_steps(&ids, 0, true), None);
    request_action(&db, Action::Continue);
    let other = db.clone();
    let mut once = false;
    let hook: Hook = Box::new(move |command| {
        if !once && command.contains("BODY.PEEK") {
            once = true;
            message_changes::update(
                &mut other.try_lock().unwrap(),
                "acc",
                "m",
                &message_changes::Patch {
                    starred: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    });
    run(&db, cross_steps(&ids, 8, false), Some(hook));
    assert_eq!(state(&db), "unresolved");
    assert!(
        !mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .starred
    );
}
#[test]
fn changed_membership_during_confirmation_requires_a_new_review() {
    let db = strong_cross();
    let first = vec![(7, true)];
    request_action(&db, Action::Check);
    run(&db, cross_steps(&first, 0, true), None);
    assert_eq!(state(&db), "located");
    request_action(&db, Action::Confirm);
    run(&db, cross_steps(&[(7, true), (8, false)], 0, true), None);
    assert_eq!(state(&db), "unresolved");
}
#[test]
fn archive_with_more_than_one_hundred_candidates_starts_a_bounded_page() {
    let db = db(false, true);
    let ids = (1..102).collect::<Vec<_>>();
    request_action(&db, Action::Check);
    let mut steps = vec![
        examine_step("INBOX", 10),
        absent(1),
        examine_step("Archive", 20),
        search(&ids),
        examine_step("INBOX", 10),
        examine_step("INBOX", 10),
        absent(1),
    ];
    steps.extend(listing("Archive", 20, &ids));
    steps.push(examine_step("Archive", 20));
    for uid in 1..9 {
        steps.extend(mail(uid, &body().replace("Body", "Other")));
    }
    steps.extend(listing("Archive", 20, &ids));
    steps.extend([examine_step("INBOX", 10), absent(1)]);
    let log = run(&db, steps, None);
    assert_eq!(state(&db), "scanning");
    assert_eq!(progress(&db).total, 101);
    assert_eq!(progress(&db).checked, 8);
    assert_eq!(
        log.iter()
            .filter(|command| command.contains("BODY.PEEK"))
            .count(),
        8
    );
}
fn cross_archive_steps(source_match: bool) -> Vec<(String, String)> {
    let mut steps = vec![examine_step("INBOX", 20), examine_step("INBOX", 20)];
    steps.extend(listing("INBOX", 20, &[3]));
    steps.extend(listing(
        "Archive",
        30,
        if source_match { &[] } else { &[7] },
    ));
    steps.push(examine_step("INBOX", 20));
    steps.extend(mail(
        3,
        &if source_match {
            body()
        } else {
            body().replace("Body", "Other")
        },
    ));
    if !source_match {
        steps.push(examine_step("Archive", 30));
        steps.extend(mail(7, &body()));
    }
    steps.extend(listing("INBOX", 20, &[3]));
    steps.extend(listing(
        "Archive",
        30,
        if source_match { &[] } else { &[7] },
    ));
    steps.push(examine_step(
        if source_match { "INBOX" } else { "Archive" },
        if source_match { 20 } else { 30 },
    ));
    steps.extend(mail(if source_match { 3 } else { 7 }, &body()));
    steps
}
#[test]
fn unknown_archive_can_rebind_target_across_generations_or_explicitly_retry_the_new_source() {
    for source_match in [false, true] {
        let db = db(false, true);
        request_action(&db, Action::Check);
        run(&db, cross_archive_steps(source_match), None);
        assert_eq!(
            state(&db),
            if source_match {
                "source-present"
            } else {
                "located"
            }
        );
        request_action(
            &db,
            if source_match {
                Action::RetryArchive
            } else {
                Action::Confirm
            },
        );
        run(&db, cross_archive_steps(source_match), None);
        assert_eq!(state(&db), "restored");
        let conn = db.lock().unwrap();
        let pending = message_changes::pending(&conn, "acc").unwrap();
        assert!(pending.iter().all(
            |intent| intent.validity == if source_match { 20 } else { 30 }
                && intent.uid == if source_match { 3 } else { 7 }
        ));
        assert_eq!(
            pending.iter().any(|intent| intent.field == "archive"),
            source_match
        );
    }
}

#[test]
fn candidates_over_the_snapshot_cap_do_not_create_partial_evidence() {
    let db = strong_cross();
    request_action(&db, Action::Check);
    let mut steps = vec![examine_step("INBOX", 20), examine_step("INBOX", 20)];
    steps.extend(listing("INBOX", 20, &(1..50_002).collect::<Vec<_>>()));
    let (mut session, _, _) = session(steps, None);
    assert!(process(&db, "acc", &mut session, &["INBOX".into()]).is_err());
    assert_eq!(state(&db), "failed");
    assert!(status(&db.lock().unwrap(), "m")
        .unwrap()
        .unwrap()
        .progress
        .is_none());
}

#[test]
fn v6_upgrade_keeps_old_review_proof_and_local_only_changes() {
    let mut conn = Connection::open_in_memory().unwrap();
    for sql in [
        include_str!("../migrations/001_foundation.sql"),
        include_str!("../migrations/002_mail_store.sql"),
        include_str!("../migrations/003_local_drafts.sql"),
        include_str!("../migrations/004_sync_generation.sql"),
        include_str!("../migrations/005_message_changes.sql"),
        include_str!("../migrations/006_remote_recovery.sql"),
    ] {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", 6).unwrap();
    message_changes::tests::seed(&conn);
    let generation: String = conn
        .query_row(
            "SELECT sync_generation FROM accounts WHERE id='acc'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let proof = serde_json::json!({"location":{"folder":"INBOX","kind":"inbox","uidValidity":10,"uid":1},"hash":hash(body().as_bytes()),"header_id":null,"result":"legacy","strength":"cached-fields","target":null});
    conn.execute("INSERT INTO message_recovery VALUES ('acc','m',?1,1,'check','located','Old proof',?2,'2026-11-11T00:00:00Z')",params![generation,proof.to_string()]).unwrap();
    conn.execute("UPDATE messages SET state_revision=1,starred=1", [])
        .unwrap();
    conn.execute("INSERT INTO message_changes(account_id,message_id,field,value,revision,state,generation,updated_at) VALUES ('acc','m','starred',1,1,'local',?1,'2026-11-11T00:00:00Z')",[generation]).unwrap();
    crate::storage::migrate(&mut conn).unwrap();
    let db = Arc::new(Mutex::new(conn));
    assert_eq!(state(&db), "located");
    request_action(&db, Action::Confirm);
    run(&db, legacy_steps(), None);
    assert_eq!(state(&db), "restored");
    let conn = db.lock().unwrap();
    assert!(
        mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .starred
    );
    assert_eq!(
        message_changes::statuses(&conn, "m").unwrap()[0].state,
        "local"
    );
}

#[test]
fn one_hundred_and_one_candidates_are_fully_scanned_twice_before_binding() {
    let db = strong_cross();
    let ids = (1..102).map(|uid| (uid, uid == 101)).collect::<Vec<_>>();
    for action in [Action::Check, Action::Confirm] {
        request_action(&db, action);
        for offset in (0..ids.len()).step_by(8) {
            if offset > 0 {
                assert_eq!(state(&db), "scanning");
                request_action(&db, Action::Continue);
            }
            run(&db, cross_steps(&ids, offset, offset == 0), None);
        }
        assert_eq!(
            state(&db),
            if matches!(action, Action::Check) {
                "located"
            } else {
                "restored"
            }
        );
    }
    let conn = db.lock().unwrap();
    let message = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert_eq!(message.remote_id.as_deref(), Some("imap:inbox:20:101"));
    assert!(message.starred);
}
