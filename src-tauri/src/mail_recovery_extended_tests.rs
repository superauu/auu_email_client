use super::*;
fn refill() -> SharedDb {
    let db = db(false, false);
    db.lock()
        .unwrap()
        .execute(
            "UPDATE messages SET content_hash=NULL,content_hash_verified=0",
            [],
        )
        .unwrap();
    db
}
fn fingerprint_steps() -> Vec<(String, String)> {
    let mut steps = vec![examine_step("INBOX", 10)];
    steps.extend(mail(1, &body()));
    steps.push(examine_step("INBOX", 10));
    steps.extend(mail(1, &body()));
    steps
}
fn run_exact(db: &SharedDb, steps: Vec<(String, String)>, hook: Option<Hook>) {
    let expected = steps.len() + 1;
    let log = run(db, steps, hook);
    assert_eq!(
        log.len(),
        expected,
        "every revalidation command must be consumed"
    );
}
#[test]
fn safe_fingerprint_refill_requires_two_checks_and_preserves_local_intents() {
    let db = refill();
    let before = message_changes::pending(&db.lock().unwrap(), "acc")
        .unwrap()
        .remove(0)
        .revision;
    request_action(&db, Action::Fingerprint);
    run_exact(&db, fingerprint_steps(), None);
    assert_eq!(state(&db), "located");
    {
        let conn = db.lock().unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(m.content_hash.is_none());
        assert_eq!(m.recovery.unwrap().operation, "fingerprint");
    }
    request_action(&db, Action::ConfirmFingerprint);
    run_exact(&db, fingerprint_steps(), None);
    let conn = db.lock().unwrap();
    let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert!(m.content_hash_verified && m.starred);
    assert_eq!(m.content_hash, Some(hash(body().as_bytes())));
    assert_eq!(m.remote_id.as_deref(), Some("imap:inbox:10:1"));
    let pending = message_changes::pending(&conn, "acc").unwrap();
    assert_eq!(pending[0].revision, before);
    assert_eq!(pending[0].uid, 1);
}
#[test]
fn missing_or_inconsistent_original_anchor_never_allows_fingerprint_refill() {
    for sql in [
        "UPDATE messages SET remote_generation=NULL",
        "UPDATE messages SET locator_verified=0",
        "UPDATE messages SET remote_uid=9",
        "UPDATE accounts SET email='changed@example.test'",
    ] {
        let db = refill();
        let mut conn = db.lock().unwrap();
        conn.execute(sql, []).unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(request(&mut conn, "acc", "m", m.state_revision, Action::Fingerprint).is_err());
        assert!(mail_store::message_by_id(&conn, "acc", "m")
            .unwrap()
            .content_hash
            .is_none());
    }
}
#[test]
fn raw_changes_or_uidvalidity_changes_do_not_fill_a_fingerprint() {
    for changed_namespace in [false, true] {
        let db = refill();
        request_action(&db, Action::Fingerprint);
        let mut steps = vec![examine_step("INBOX", 10)];
        steps.extend(mail(1, &body()));
        steps.push(examine_step(
            "INBOX",
            if changed_namespace { 11 } else { 10 },
        ));
        if !changed_namespace {
            steps.extend(mail(
                1,
                &body().replace("Subject: Test", "X-Extra: changed\r\nSubject: Test"),
            ));
        }
        run_exact(&db, steps, None);
        assert_eq!(state(&db), "unresolved");
        assert!(mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .content_hash
            .is_none());
    }
}
#[test]
fn weak_content_recovery_cannot_be_promoted_to_a_trusted_historical_fingerprint() {
    let db = db(true, false);
    request_action(&db, Action::Check);
    run_exact(&db, legacy_steps(), None);
    request_action(&db, Action::Confirm);
    run_exact(&db, legacy_steps(), None);
    {
        let mut conn = db.lock().unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(m.content_hash.is_some());
        assert!(!m.content_hash_verified && !m.locator_verified);
        assert!(request(&mut conn, "acc", "m", m.state_revision, Action::Fingerprint).is_err());
        conn.execute("UPDATE message_changes SET state='conflict'", [])
            .unwrap();
    }
    request_action(&db, Action::Check);
    run_exact(&db, vec![examine_step("INBOX", 20)], None);
    assert_eq!(state(&db), "unresolved");
}
#[test]
fn fingerprint_confirmation_rejects_edits_and_sql_failure() {
    for fail_sql in [false, true] {
        let db = refill();
        request_action(&db, Action::Fingerprint);
        run_exact(&db, fingerprint_steps(), None);
        request_action(&db, Action::ConfirmFingerprint);
        if fail_sql {
            db.lock().unwrap().execute_batch("CREATE TRIGGER fail_hash BEFORE UPDATE OF content_hash ON messages BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
            let (mut s, _, _) = session(fingerprint_steps(), None);
            assert!(process(&db, "acc", &mut s, &["INBOX".into()]).is_err());
        } else {
            let other = db.clone();
            let mut once = false;
            let hook: Hook = Box::new(move |cmd| {
                if !once && cmd.contains("BODY.PEEK") {
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
            run_exact(&db, fingerprint_steps(), Some(hook));
        }
        let conn = db.lock().unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(!m.content_hash_verified);
        assert_eq!(m.starred, fail_sql);
        assert_eq!(state_string(&conn), "unresolved");
    }
}

fn boundary(next: u32) -> (String, String) {
    step("EXAMINE \"INBOX\"",&format!("* 1 EXISTS\r\n* OK [UIDVALIDITY 20] valid\r\n* OK [UIDNEXT {next}] next\r\n{{tag}} OK [READ-ONLY] examined\r\n"))
}
fn range(start: u32, end: u32, ids: &[u32]) -> (String, String) {
    let list = ids
        .iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    step(
        &format!("UID SEARCH UID {start}:{end} ALL"),
        &format!(
            "* SEARCH{}\r\n{{tag}} OK search\r\n",
            if list.is_empty() {
                String::new()
            } else {
                format!(" {list}")
            }
        ),
    )
}
fn collect(ids: &[(u32, bool)], second: bool, initial: bool) -> Vec<(String, String)> {
    let mut steps = vec![boundary(50002)];
    if initial {
        steps.push(boundary(50002));
    }
    let (start, end) = if second { (50001, 50001) } else { (1, 50000) };
    let uids = ids.iter().map(|(uid, _)| *uid).collect::<Vec<_>>();
    steps.extend([boundary(50002), range(start, end, &uids)]);
    for (uid, matching) in ids.iter().take(8) {
        steps.extend(mail(
            *uid,
            &if *matching {
                body()
            } else {
                body().replace("Body", "Different")
            },
        ));
    }
    steps.extend([boundary(50002), range(start, end, &uids), boundary(50002)]);
    steps
}
fn validate_segment(ids: &[u32], second: bool, final_match: bool) -> Vec<(String, String)> {
    let (start, end) = if second { (50001, 50001) } else { (1, 50000) };
    let mut steps = vec![
        boundary(50002),
        boundary(50002),
        range(start, end, ids),
        boundary(50002),
    ];
    if final_match {
        steps.push(boundary(50002));
        steps.extend(mail(50001, &body()));
        steps.push(boundary(50002));
    }
    steps
}
fn strong_segments() -> SharedDb {
    let db = db(false, false);
    db.lock()
        .unwrap()
        .execute("UPDATE message_changes SET state='conflict'", [])
        .unwrap();
    db
}
fn finish_segmented_check(db: &SharedDb) {
    run_exact(db, collect(&[(1, false), (2, false)], false, true), None);
    request_action(db, Action::Continue);
    run_exact(db, collect(&[(50001, true)], true, false), None);
    request_action(db, Action::Continue);
    run_exact(db, validate_segment(&[1, 2], false, false), None);
    request_action(db, Action::Continue);
    run_exact(db, validate_segment(&[50001], true, true), None);
}
#[test]
fn segmented_recovery_revalidates_all_ranges_and_repeats_on_confirmation() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    finish_segmented_check(&db);
    assert_eq!(state(&db), "located");
    request_action(&db, Action::Confirm);
    finish_segmented_check(&db);
    assert_eq!(state(&db), "restored");
    let conn = db.lock().unwrap();
    let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert_eq!(m.remote_id.as_deref(), Some("imap:inbox:20:50001"));
    assert!(m.starred && m.content_hash_verified);
}
#[test]
fn more_than_fifty_thousand_candidates_do_not_require_a_global_snapshot() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    let ids = (1..50001).map(|uid| (uid, false)).collect::<Vec<_>>();
    run_exact(&db, collect(&ids, false, true), None);
    let status = status(&db.lock().unwrap(), "m").unwrap().unwrap();
    assert_eq!(status.state, "scanning");
    let p = status.progress.unwrap();
    assert_eq!(p.total, 50000);
    assert_eq!(p.checked, 8);
    assert_eq!(p.segments.unwrap().total, 2);
    assert!(status.location.is_none());
}
fn dense_page(begin: u32, exists: u32) -> Vec<(String, String)> {
    let boundary = || {
        let (command, reply) = boundary(50002);
        (
            command,
            reply.replace("* 1 EXISTS", &format!("* {exists} EXISTS")),
        )
    };
    let ids = (1..=1024).collect::<Vec<_>>();
    let mut steps = vec![boundary()];
    if begin == 1 {
        steps.push(boundary());
    }
    steps.extend([boundary(), range(1, 1024, &ids)]);
    for uid in begin..begin + 8 {
        steps.extend(mail(uid, &body().replace("Body", "Different")));
    }
    steps.extend([boundary(), range(1, 1024, &ids), boundary()]);
    steps
}
#[test]
fn dense_mailbox_uses_small_stable_ranges_without_reducing_page_revalidation() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    let unlocked = db.clone();
    let hook: Hook = Box::new(move |_| {
        assert!(unlocked.try_lock().is_ok());
    });
    run_exact(&db, dense_page(1, 50001), Some(hook));
    let checkpoint: String = db
        .lock()
        .unwrap()
        .query_row("SELECT checkpoint_json FROM recovery_scan", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        checkpoint.len() < 10_000,
        "dense page must not persist 50000 UIDs"
    );
    let json: serde_json::Value = serde_json::from_str(&checkpoint).unwrap();
    assert_eq!(json["fences"][0]["span"], 1024);
    assert_eq!(json["current"]["uids"].as_array().unwrap().len(), 1024);
    // 6250 pages of a 50000-UID segment formerly retransmitted 312.5 million
    // IDs twice. Fixed 1024-UID intervals preserve both checks with <=128 pages.
    assert_eq!(
        status(&db.lock().unwrap(), "m")
            .unwrap()
            .unwrap()
            .progress
            .unwrap()
            .segments
            .unwrap()
            .total,
        49
    );
    request_action(&db, Action::Continue);
    // EXISTS can decrease without changing UIDNEXT. Never resize a saved interval.
    run_exact(&db, dense_page(9, 100), None);
    let m = mail_store::message_by_id(&db.lock().unwrap(), "acc", "m").unwrap();
    assert!(m.starred);
    assert_eq!(m.remote_id.as_deref(), Some("imap:inbox:10:1"));
    assert_eq!(m.recovery.unwrap().progress.unwrap().checked, 16);
}
#[test]
fn dense_mailboxes_below_the_old_snapshot_cap_also_use_small_segments() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    let steps = dense_page(1, 4096)
        .into_iter()
        .map(|(command, reply)| (command, reply.replace("UIDNEXT 50002", "UIDNEXT 4097")))
        .collect();
    run_exact(&db, steps, None);
    let progress = status(&db.lock().unwrap(), "m")
        .unwrap()
        .unwrap()
        .progress
        .unwrap();
    assert_eq!(progress.checked, 8);
    assert_eq!(progress.total, 1024);
    assert_eq!(progress.segments.unwrap().total, 4);
}
#[test]
fn header_hint_does_not_turn_a_dense_mailbox_into_many_empty_intervals() {
    let db = strong_segments();
    db.lock()
        .unwrap()
        .execute(
            "UPDATE messages SET header_message_id='<hint@example.test>'",
            [],
        )
        .unwrap();
    request_action(&db, Action::Check);
    let steps = collect(&[(1, false)], false, true)
        .into_iter()
        .map(|(command, reply)| {
            (
                command.replace(" ALL", " HEADER Message-ID \"<hint@example.test>\""),
                reply.replace("* 1 EXISTS", "* 50001 EXISTS"),
            )
        })
        .collect();
    run_exact(&db, steps, None);
    let conn = db.lock().unwrap();
    let progress = status(&conn, "m").unwrap().unwrap().progress.unwrap();
    assert_eq!(progress.segments.unwrap().total, 2);
    let json: String = conn
        .query_row("SELECT checkpoint_json FROM recovery_scan", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        !json.contains("\"span\""),
        "default width must preserve the legacy checkpoint shape"
    );
}
#[test]
fn dense_page_membership_change_and_invalid_saved_width_never_bind() {
    for invalid_width in [false, true] {
        let db = strong_segments();
        request_action(&db, Action::Check);
        run_exact(&db, dense_page(1, 50001), None);
        let mut steps = dense_page(9, 50001);
        if invalid_width {
            let conn = db.lock().unwrap();
            let json: String = conn
                .query_row("SELECT checkpoint_json FROM recovery_scan", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let mut json: serde_json::Value = serde_json::from_str(&json).unwrap();
            json["fences"][0]["span"] = 0.into();
            conn.execute(
                "UPDATE recovery_scan SET checkpoint_json=?1",
                [json.to_string()],
            )
            .unwrap();
            steps.clear();
        } else {
            steps.truncate(3);
            steps[2] = range(1, 1024, &(2..=1024).collect::<Vec<_>>());
        }
        request_action(&db, Action::Continue);
        run_exact(&db, steps, None);
        let m = mail_store::message_by_id(&db.lock().unwrap(), "acc", "m").unwrap();
        assert!(m.starred);
        assert_eq!(m.remote_id.as_deref(), Some("imap:inbox:10:1"));
        assert_eq!(m.recovery.unwrap().state, "unresolved");
    }
}
#[test]
fn streaming_failure_keeps_previous_checkpoint_and_local_intents_for_explicit_continuation() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    run_exact(&db, collect(&[(1, false)], false, true), None);
    let saved: String = db
        .lock()
        .unwrap()
        .query_row("SELECT checkpoint_json FROM recovery_scan", [], |r| {
            r.get(0)
        })
        .unwrap();
    request_action(&db, Action::Continue);
    let mut steps = vec![
        boundary(50002),
        boundary(50002),
        range(50001, 50001, &[50001]),
    ];
    steps.push(step(
        "UID FETCH 50001 (UID RFC822.SIZE)",
        "* 1 FETCH (UID 50001 RFC822.SIZE 20)\r\n{tag} OK size\r\n",
    ));
    steps.push(step(
        "UID FETCH 50001 (UID INTERNALDATE BODY.PEEK[])",
        "* 1 FETCH (UID 50001 BODY[] {99999999999999}\r\n",
    ));
    let (mut s, _, _) = session(steps, None);
    assert!(process(&db, "acc", &mut s, &["INBOX".into()]).is_err());
    {
        let conn = db.lock().unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert!(m.starred && m.content_hash_verified);
        assert_eq!(m.remote_id.as_deref(), Some("imap:inbox:10:1"));
        let recovery = m.recovery.unwrap();
        assert_eq!(recovery.state, "failed");
        assert!(recovery.detail.contains("安全限额"));
        assert_eq!(recovery.progress.unwrap().checked, 1);
        assert_eq!(
            conn.query_row("SELECT checkpoint_json FROM recovery_scan", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            saved
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM message_changes WHERE state='conflict'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    request_action(&db, Action::Continue);
    run_exact(&db, collect(&[(50001, true)], true, false), None);
    assert_eq!(state(&db), "scanning");
}
#[test]
fn duplicate_in_a_different_segment_does_not_allow_binding() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    run_exact(&db, collect(&[(1, true)], false, true), None);
    request_action(&db, Action::Continue);
    let mut steps = vec![
        boundary(50002),
        boundary(50002),
        range(50001, 50001, &[50001]),
    ];
    steps.extend(mail(50001, &body()));
    run_exact(&db, steps, None);
    assert_eq!(state(&db), "unresolved");
    assert_eq!(
        mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .remote_id
            .as_deref(),
        Some("imap:inbox:10:1")
    );
}
#[test]
fn deletion_in_an_already_collected_segment_is_caught_by_the_final_pass() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    run_exact(&db, collect(&[(1, false), (2, false)], false, true), None);
    request_action(&db, Action::Continue);
    run_exact(&db, collect(&[(50001, true)], true, false), None);
    request_action(&db, Action::Continue);
    run_exact(&db, validate_segment(&[2], false, false), None);
    assert_eq!(state(&db), "scanning");
    request_action(&db, Action::Continue);
    run_exact(&db, validate_segment(&[50001], true, false), None);
    assert_eq!(state(&db), "unresolved");
}
#[test]
fn segmented_checks_stop_on_namespace_or_range_errors_and_retain_edits() {
    for invalid in [false, true] {
        let db = strong_segments();
        request_action(&db, Action::Check);
        if invalid {
            run_exact(
                &db,
                vec![
                    boundary(50002),
                    boundary(50002),
                    boundary(50002),
                    range(1, 50000, &[50001]),
                ],
                None,
            );
        } else {
            run_exact(&db, collect(&[(1, false)], false, true), None);
            request_action(&db, Action::Continue);
            run_exact(&db, vec![boundary(50003)], None);
        }
        assert_eq!(state(&db), "unresolved");
        assert!(
            mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
                .unwrap()
                .starred
        );
    }
}

#[test]
fn segmented_checkpoint_survives_reopen_and_network_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segments.sqlite");
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
    conn.execute("UPDATE message_changes SET state='conflict'", [])
        .unwrap();
    let db = Arc::new(Mutex::new(conn));
    request_action(&db, Action::Check);
    run_exact(&db, collect(&[(1, false)], false, true), None);
    drop(db);
    let db = Arc::new(Mutex::new(crate::storage::initialize(&path).unwrap()));
    assert_eq!(state(&db), "scanning");
    request_action(&db, Action::Continue);
    let (mut s, _, _) = session(vec![step("EXAMINE", "")], None);
    assert!(process(&db, "acc", &mut s, &["INBOX".into()]).is_err());
    assert_eq!(state(&db), "failed");
    assert_eq!(
        status(&db.lock().unwrap(), "m")
            .unwrap()
            .unwrap()
            .progress
            .unwrap()
            .segments
            .unwrap()
            .completed,
        1
    );
    request_action(&db, Action::Continue);
    run_exact(&db, collect(&[(50001, true)], true, false), None);
    assert_eq!(state(&db), "scanning");
    assert!(
        mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .starred
    );
}
#[test]
fn a_saved_cross_generation_confirmation_cannot_bypass_downgraded_provenance() {
    let db = strong_segments();
    request_action(&db, Action::Check);
    finish_segmented_check(&db);
    request_action(&db, Action::Confirm);
    run_exact(&db, collect(&[(1, false), (2, false)], false, true), None);
    db.lock()
        .unwrap()
        .execute(
            "UPDATE messages SET content_hash_verified=0,locator_verified=0",
            [],
        )
        .unwrap();
    request_action(&db, Action::Continue);
    run_exact(&db, vec![], None);
    assert_eq!(state(&db), "unresolved");
    assert_eq!(
        mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .remote_id
            .as_deref(),
        Some("imap:inbox:10:1")
    );
}
#[test]
fn v7_upgrade_does_not_promote_past_recovery_observations_to_trusted_history() {
    for recovered in [false, true] {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in [
            include_str!("../migrations/001_foundation.sql"),
            include_str!("../migrations/002_mail_store.sql"),
            include_str!("../migrations/003_local_drafts.sql"),
            include_str!("../migrations/004_sync_generation.sql"),
            include_str!("../migrations/005_message_changes.sql"),
            include_str!("../migrations/006_remote_recovery.sql"),
            include_str!("../migrations/007_recovery_scan.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 7).unwrap();
        message_changes::tests::seed(&conn);
        conn.execute(
            "UPDATE messages SET content_hash=?1",
            [hash(body().as_bytes())],
        )
        .unwrap();
        if recovered {
            conn.execute("INSERT INTO message_recovery SELECT 'acc','m',sync_generation,0,'check','restored','Old weak result','{\"strength\":\"cached-fields\"}','2026-11-11T00:00:00Z' FROM accounts WHERE id='acc'",[]).unwrap();
        }
        crate::storage::migrate(&mut conn).unwrap();
        let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
        assert_eq!(m.content_hash_verified, !recovered);
        assert_eq!(m.locator_verified, !recovered);
        assert_eq!(m.body_text, "Body");
    }
}
