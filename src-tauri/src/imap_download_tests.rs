use super::*;
use crate::imap_writeback::tests::{session, step};
fn fence(validity: u32) -> (String, String) {
    step(
        "EXAMINE",
        &format!("* OK [UIDVALIDITY {validity}] valid\r\n{{tag}} OK examine\r\n"),
    )
}
fn raw(size: usize) -> Vec<u8> {
    let mut bytes =
        b"Subject: Test\r\nFrom: s@example.test\r\nContent-Type: text/plain\r\n\r\n".to_vec();
    bytes.resize(size, b'x');
    bytes
}
fn fetch(uid: u32, raw: &[u8]) -> String {
    format!("* 1 FETCH (UID {uid} RFC822.SIZE {} FLAGS (\\Seen) INTERNALDATE \"11-Nov-2026 00:00:00 +0000\" BODY[] {{{}}}\r\n{})\r\n",raw.len(),raw.len(),std::str::from_utf8(raw).unwrap())
}
#[test]
fn planned_fetch_validates_size_and_namespace_before_returning_verified_cached_raw() {
    let raw = raw(200);
    let dir = tempfile::tempdir().unwrap();
    for changed in [false, true] {
        let (mut s, _, _) = session(
            vec![
                step(
                    "RFC822.SIZE",
                    "* 1 FETCH (UID 1 RFC822.SIZE 200)\r\n{tag} OK sizes\r\n",
                ),
                step(
                    "UID FETCH 1 (UID FLAGS",
                    &(fetch(1, &raw) + "{tag} OK body\r\n"),
                ),
                fence(if changed { 2 } else { 1 }),
            ],
            None,
        );
        let result = fetch_range(&mut s, 1, 1, "INBOX", 1, dir.path());
        if changed {
            assert_eq!(result.err().unwrap().code, "conflict");
        } else {
            let mail = result.unwrap().remove(0);
            assert_eq!(
                crate::mail_cache::read(dir.path(), mail.raw_cache_key.as_ref().unwrap()).unwrap(),
                raw
            );
        }
    }
}
#[test]
fn size_planning_splits_batches_without_retrying_a_poisoned_stream() {
    let raw = raw(3 * 1024 * 1024);
    let dir = tempfile::tempdir().unwrap();
    let (mut s,_,log)=session(vec![
        step("RFC822.SIZE", &format!("* 1 FETCH (UID 1 RFC822.SIZE {0})\r\n* 2 FETCH (UID 2 RFC822.SIZE {0})\r\n* 3 FETCH (UID 3 RFC822.SIZE {0})\r\n{{tag}} OK sizes\r\n",raw.len())),
        step("UID FETCH 1,2 ",&(fetch(1,&raw)+&fetch(2,&raw)+"{tag} OK body\r\n")),step("UID FETCH 3 ",&(fetch(3,&raw)+"{tag} OK body\r\n")),fence(1)
    ],None);
    assert_eq!(
        fetch_range(&mut s, 1, 3, "INBOX", 1, dir.path())
            .unwrap()
            .len(),
        3
    );
    assert_eq!(log.lock().unwrap().len(), 5);
}
#[test]
fn partial_download_assembles_more_than_one_literal_limit_with_exact_offsets() {
    let raw = raw(17 * 1024 * 1024 + 3);
    let size = raw.len() as u32;
    let dir = tempfile::tempdir().unwrap();
    let mut steps = vec![step(
        "RFC822.SIZE",
        &format!("* 1 FETCH (UID 1 RFC822.SIZE {size})\r\n{{tag}} OK size\r\n"),
    )];
    for offset in (0..size).step_by(CHUNK_BYTES as usize) {
        let count = CHUNK_BYTES.min(size - offset);
        let bytes = &raw[offset as usize..(offset + count) as usize];
        steps.push(step(
            &format!("BODY.PEEK[]<{offset}.{count}>"),
            &format!(
                "* 1 FETCH (UID 1 BODY[]<{offset}> {{{count}}}\r\n{})\r\n{{tag}} OK chunk\r\n",
                std::str::from_utf8(bytes).unwrap()
            ),
        ));
    }
    steps.push(step("RFC822.SIZE",&format!("* 1 FETCH (UID 1 RFC822.SIZE {size} FLAGS (\\Seen) INTERNALDATE \"11-Nov-2026 00:00:00 +0000\")\r\n{{tag}} OK metadata\r\n")));
    steps.push(fence(1));
    let (mut s, _, _) = session(steps, None);
    let mail = fetch_range(&mut s, 1, 1, "INBOX", 1, dir.path())
        .unwrap()
        .remove(0);
    assert_eq!(mail.content_hash, crate::mail_recovery::hash(&raw));
    assert_eq!(
        crate::mail_cache::read(dir.path(), mail.raw_cache_key.as_ref().unwrap()).unwrap(),
        raw
    );
}
#[test]
fn invalid_chunks_and_oversize_candidates_never_become_partial_success() {
    for response in [
        "* 1 FETCH (UID 1 BODY[]<1> {3}\r\nabc)\r\n",
        "* 1 FETCH (UID 2 BODY[]<0> {3}\r\nabc)\r\n",
        "* 1 FETCH (UID 1 BODY[]<0> {2}\r\nab)\r\n",
        "* 1 FETCH (UID 1 BODY[]<0> {3}\r\nab",
        "* 1 FETCH (UID 1 BODY[]<0> {3}\r\nabc BODY[]<0> {3}\r\nabc)\r\n",
    ] {
        assert!(chunk(response.as_bytes(), 1, 0, 3).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _, log) = session(
        vec![step(
            "RFC822.SIZE",
            &format!(
                "* 1 FETCH (UID 1 RFC822.SIZE {})\r\n{{tag}} OK size\r\n",
                MAX_RAW + 1
            ),
        )],
        None,
    );
    assert!(fetch_range(&mut s, 1, 1, "INBOX", 1, dir.path()).is_err());
    assert_eq!(log.lock().unwrap().len(), 2);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn a_partial_network_failure_removes_the_incomplete_spool() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _, _) = session(
        vec![
            step(
                "RFC822.SIZE",
                "* 1 FETCH (UID 1 RFC822.SIZE 9000000)\r\n{tag} OK size\r\n",
            ),
            step("BODY.PEEK[]<0.262144>", ""),
        ],
        None,
    );
    assert!(fetch_range(&mut s, 1, 1, "INBOX", 1, dir.path()).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
