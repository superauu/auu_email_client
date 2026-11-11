use super::*;
fn fixture() -> (rusqlite::Connection, String) {
    let mut conn = crate::message_changes::tests::fixture();
    request(&mut conn, &["acc".into()]).unwrap();
    let generation = conn
        .query_row(
            "SELECT sync_generation FROM accounts WHERE id='acc'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    (conn, generation)
}
fn state(conn: &rusqlite::Connection) -> (String, String, u32) {
    conn.query_row(
        "SELECT state,due_at,attempts FROM imap_sync_requests WHERE account_id='acc'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}
#[test]
fn network_backoff_is_bounded_and_auth_cancel_or_changed_generation_pause() {
    let now = "2026-11-11T00:00:00Z";
    for attempts in [1, 4, 8] {
        let (conn, generation) = fixture();
        conn.execute(
            "UPDATE imap_sync_requests SET state='running',attempts=?1",
            [attempts],
        )
        .unwrap();
        complete(
            &conn,
            "acc",
            &generation,
            &Err(AppError::network("offline")),
            now,
        )
        .unwrap();
        let result = state(&conn);
        assert_eq!(result.0, if attempts == 8 { "paused" } else { "failed" });
        if attempts == 1 {
            assert_eq!(result.1, "2026-11-11T00:00:05Z");
        }
    }
    for change in [false, true] {
        let (conn, generation) = fixture();
        if change {
            conn.execute(
                "UPDATE accounts SET email='changed@example.test' WHERE id='acc'",
                [],
            )
            .unwrap();
        }
        complete(
            &conn,
            "acc",
            &generation,
            &Err(AppError::auth("login refused")),
            now,
        )
        .unwrap();
        assert_eq!(state(&conn).0, "paused");
    }
}
#[test]
fn completed_work_continues_only_when_requested_and_success_honors_latest_auto_setting() {
    let now = "2026-11-11T00:00:00Z";
    let (conn, generation) = fixture();
    complete(
        &conn,
        "acc",
        &generation,
        &Ok(SyncOutcome {
            has_more: true,
            ..Default::default()
        }),
        now,
    )
    .unwrap();
    assert_eq!(state(&conn).1, "2026-11-11T00:00:01Z");
    crate::imap_settings::save(
        &conn,
        "acc",
        &crate::imap_settings::Settings {
            auto_sync_seconds: 60,
            ..Default::default()
        },
    )
    .unwrap();
    complete(&conn, "acc", &generation, &Ok(SyncOutcome::default()), now).unwrap();
    assert_eq!(state(&conn).1, "2026-11-11T00:01:00Z");
    crate::imap_settings::save(&conn, "acc", &Default::default()).unwrap();
    complete(&conn, "acc", &generation, &Ok(SyncOutcome::default()), now).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM imap_sync_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn an_old_completion_cannot_delete_a_request_for_a_reconfigured_account() {
    let (mut conn, old) = fixture();
    conn.execute(
        "UPDATE accounts SET email='new@example.test' WHERE id='acc'",
        [],
    )
    .unwrap();
    request(&mut conn, &["acc".into()]).unwrap();
    complete(
        &conn,
        "acc",
        &old,
        &Ok(SyncOutcome::default()),
        "2026-11-11T00:00:00Z",
    )
    .unwrap();
    assert_eq!(state(&conn).0, "queued");
    pause(&conn, "acc").unwrap();
    complete(
        &conn,
        "acc",
        &old,
        &Ok(SyncOutcome {
            has_more: true,
            ..Default::default()
        }),
        "2026-11-11T00:00:00Z",
    )
    .unwrap();
    assert_eq!(state(&conn).0, "paused");
}
#[test]
fn running_requests_recover_without_losing_local_unknown_moves() {
    let (mut conn, _) = fixture();
    crate::message_changes::update(
        &mut conn,
        "acc",
        "m",
        &crate::message_changes::Patch {
            archive: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    conn.execute(
        "UPDATE message_changes SET state='unknown' WHERE field='archive'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE imap_sync_requests SET state='running',attempts=3",
        [],
    )
    .unwrap();
    recover(&conn).unwrap();
    assert_eq!(state(&conn).0, "queued");
    assert_eq!(state(&conn).2, 3);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM message_changes WHERE field='archive'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
}
#[test]
fn a_new_manual_request_survives_an_old_cancelled_completion() {
    let (mut conn, generation) = fixture();
    conn.execute("UPDATE imap_sync_requests SET state='running'", [])
        .unwrap();
    let old: i64 = conn
        .query_row("SELECT request_version FROM imap_sync_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    pause(&conn, "acc").unwrap();
    request(&mut conn, &["acc".into()]).unwrap();
    complete_version(
        &conn,
        "acc",
        &generation,
        Some(old),
        &Err(AppError::invalid("old worker stopped")),
        "2026-11-11T00:00:00Z",
    )
    .unwrap();
    assert_eq!(state(&conn).0, "queued");
    conn.execute("UPDATE imap_sync_requests SET state='running'", [])
        .unwrap();
    let old: i64 = conn
        .query_row("SELECT request_version FROM imap_sync_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    request(&mut conn, &["acc".into()]).unwrap();
    complete_version(
        &conn,
        "acc",
        &generation,
        Some(old),
        &Ok(SyncOutcome::default()),
        "2026-11-11T00:00:00Z",
    )
    .unwrap();
    assert_eq!(state(&conn).0, "queued");
}
