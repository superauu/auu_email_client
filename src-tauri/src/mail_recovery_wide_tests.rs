use super::*;
fn boundary(name: &str, validity: u32) -> (String, String) {
    step(&format!("EXAMINE \"{name}\""),&format!("* 1 EXISTS\r\n* OK [UIDVALIDITY {validity}] valid\r\n* OK [UIDNEXT 2] next\r\n{{tag}} OK examined\r\n"))
}
fn both() -> Vec<(String, String)> {
    vec![boundary("INBOX", 10), boundary("Archive", 20)]
}
fn search(found: bool) -> (String, String) {
    step(
        "UID SEARCH UID 1:1 ALL",
        if found {
            "* SEARCH 1\r\n{tag} OK search\r\n"
        } else {
            "* SEARCH\r\n{tag} OK search\r\n"
        },
    )
}
fn collect_first(source_present: bool) -> Vec<(String, String)> {
    let mut steps = both();
    steps.extend([boundary("INBOX", 10), search(source_present)]);
    if source_present {
        steps.extend(mail(1, &body()));
    }
    steps.extend([boundary("INBOX", 10), search(source_present)]);
    steps.extend(both());
    steps
}
fn collect_second(duplicate: bool) -> Vec<(String, String)> {
    let mut steps = both();
    steps.extend([boundary("Archive", 20), search(true)]);
    steps.extend(mail(1, &body()));
    if !duplicate {
        steps.extend([boundary("Archive", 20), search(true)]);
        steps.extend(both());
    }
    steps
}
fn validate_first() -> Vec<(String, String)> {
    let mut steps = both();
    steps.extend([boundary("INBOX", 10), search(false)]);
    steps.extend(both());
    steps
}
fn validate_last() -> Vec<(String, String)> {
    let mut steps = both();
    steps.extend([boundary("Archive", 20), search(true)]);
    steps.extend(both());
    steps.push(examine_step("Archive", 20));
    steps.extend(mail(1, &body()));
    steps.extend(both());
    steps
}
fn fixture() -> SharedDb {
    let db = db(false, false);
    db.lock()
        .unwrap()
        .execute("UPDATE messages SET remote_missing=1", [])
        .unwrap();
    db
}
fn inspect_all(db: &SharedDb) {
    run(db, collect_first(false), None);
    request_action(db, Action::Continue);
    run(db, collect_second(false), None);
    request_action(db, Action::Continue);
    run(db, validate_first(), None);
    request_action(db, Action::Continue);
    run(db, validate_last(), None);
}
#[test]
fn external_move_requires_all_accessible_folders_and_a_second_complete_scan() {
    let db = fixture();
    request_action(&db, Action::Check);
    inspect_all(&db);
    assert_eq!(state(&db), "located");
    assert_eq!(
        mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .remote_id
            .as_deref(),
        Some("imap:inbox:10:1")
    );
    request_action(&db, Action::Confirm);
    inspect_all(&db);
    let conn = db.lock().unwrap();
    let m = mail_store::message_by_id(&conn, "acc", "m").unwrap();
    assert_eq!(
        m.remote_id.as_deref(),
        Some(format!("imap:{}:20:1", crate::imap_settings::scope("Archive")).as_str())
    );
    assert!(!m.remote_missing && m.starred && m.content_hash_verified);
    assert_eq!(m.recovery.unwrap().state, "restored");
}
#[test]
fn cross_folder_duplicates_untrusted_hashes_and_new_directories_never_bind() {
    let db = fixture();
    request_action(&db, Action::Check);
    run(&db, collect_first(true), None);
    request_action(&db, Action::Continue);
    run(&db, collect_second(true), None);
    assert_eq!(state(&db), "unresolved");
    let db = fixture();
    db.lock()
        .unwrap()
        .execute("UPDATE messages SET content_hash_verified=0", [])
        .unwrap();
    request_action(&db, Action::Check);
    run(&db, vec![], None);
    assert_eq!(state(&db), "unresolved");
    let db = fixture();
    request_action(&db, Action::Check);
    run(&db, collect_first(false), None);
    request_action(&db, Action::Continue);
    let (mut s, _, _) = session(both(), None);
    process(
        &db,
        "acc",
        &mut s,
        &["INBOX".into(), "Archive".into(), "New".into()],
    )
    .unwrap();
    assert_eq!(state(&db), "unresolved");
}
#[test]
fn automatic_pages_stop_for_explicit_confirmation_and_recheck_every_folder() {
    let db = fixture();
    request_action(&db, Action::Check);
    run(&db, collect_first(false), None);
    request_action(&db, Action::AutoContinue);
    run(&db, collect_second(false), None);
    assert_eq!(state(&db), "queued");
    run(&db, validate_first(), None);
    assert_eq!(state(&db), "queued");
    run(&db, validate_last(), None);
    assert_eq!(state(&db), "located");
    assert_eq!(
        mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .remote_id
            .as_deref(),
        Some("imap:inbox:10:1")
    );
    request_action(&db, Action::Confirm);
    for steps in [
        collect_first(false),
        collect_second(false),
        validate_first(),
        validate_last(),
    ] {
        run(&db, steps, None);
    }
    assert_eq!(state(&db), "restored");
    assert!(
        mail_store::message_by_id(&db.lock().unwrap(), "acc", "m")
            .unwrap()
            .starred
    );
}
