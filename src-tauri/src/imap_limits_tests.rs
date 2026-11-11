use super::*;
use crate::imap_writeback::tests::{session, step};
use std::io::Cursor;
#[test]
fn sasl_and_idle_continuations_cannot_replenish_the_command_budget() {
    let mut stream = LimitedStream::new(Cursor::new(vec![]));
    stream.budget.bytes = 100;
    for line in [b"dXNlcj1maXh0dXJl\r\n".as_slice(), b"\r\n", b"DONE\r\n"] {
        stream.write_all(line).unwrap();
        stream.flush().unwrap();
        assert_eq!(stream.budget.bytes, 100);
    }
    stream.write_all(b"a123 NOOP\r\n").unwrap();
    stream.flush().unwrap();
    assert_eq!(stream.budget.bytes, 0);
}
#[test]
fn slow_responses_have_a_command_deadline_without_long_sleeps() {
    let mut stream = LimitedStream::new(Cursor::new(b"* OK ready\r\n".to_vec()));
    stream.started = std::time::Instant::now() - std::time::Duration::from_secs(61);
    assert!(stream.read(&mut [0; 32]).is_err());
    assert!(stream.write_all(b"a1 NOOP\r\n").is_err());
}

struct HeaderOnly {
    header: Cursor<Vec<u8>>,
}
impl Read for HeaderOnly {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        assert!(
            (self.header.position() as usize) < self.header.get_ref().len(),
            "oversized payload must never be requested"
        );
        // Deliberately split even the literal length across socket reads.
        self.header.read(&mut bytes[..1])
    }
}
impl Write for HeaderOnly {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("poisoned connection must not write");
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("poisoned connection must not flush");
    }
}
#[test]
fn huge_or_overflowing_literal_is_rejected_before_reading_payload_and_poison_is_permanent() {
    for size in [
        (MAX_LITERAL + 1).to_string(),
        "184467440737095516160000000000000".into(),
        format!("{}+", MAX_LITERAL + 1),
    ] {
        let header = format!("* 1 FETCH (UID 1 BODY[] {{{size}}}\r\n");
        let mut stream = LimitedStream::new(HeaderOnly {
            header: Cursor::new(header.into_bytes()),
        });
        let mut received = vec![];
        assert_eq!(
            stream.read_to_end(&mut received).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(
            !received.ends_with(b"\n"),
            "library must not receive the complete bad declaration"
        );
        assert!(stream.write_all(b"a2 LOGOUT\r\n").is_err());
        assert!(stream.flush().is_err());
        assert!(stream.read(&mut [0; 1]).is_err());
    }
}
#[test]
fn fragmented_literals_ignore_fake_protocol_and_large_body_lines() {
    let body = format!(
        "* SEARCH {}\r\n* OK [COPYUID 1 2 3]\r\n{}",
        "1 ".repeat(MAX_SEARCH_UIDS + 1),
        "x".repeat(MAX_LINE + 1)
    );
    let bytes = format!(
        "* 1 FETCH (UID 1 BODY[] {{{}}}\r\n{body})\r\na1 OK done\r\n",
        body.len()
    );
    let mut stream = LimitedStream::new(Cursor::new(bytes.as_bytes()));
    let mut result = vec![];
    let mut buffer = [0; 137];
    loop {
        let n = stream.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        result.extend_from_slice(&buffer[..n]);
    }
    assert_eq!(result, bytes.as_bytes());
}
#[test]
fn protocol_line_without_newline_has_a_streaming_bound() {
    let mut stream = LimitedStream::new(Cursor::new(vec![b'x'; MAX_LINE + 8192]));
    let mut output = vec![];
    assert!(stream.read_to_end(&mut output).is_err());
    assert!(output.len() <= MAX_LINE);
    assert!((stream.inner.position() as usize) <= MAX_LINE + 8192);
}
#[test]
fn search_count_is_bounded_across_lines_including_duplicate_uids() {
    for split in [false, true] {
        let prefix = if split {
            format!("* SEARCH {}\r\n* SEARCH", "1 ".repeat(30_000).trim_end())
        } else {
            "* SEARCH".into()
        };
        let count = if split { 20_001 } else { 50_001 };
        let (mut s, _, _) = session(
            vec![step(
                "UID SEARCH",
                &format!(
                    "{prefix} {}\r\n{{tag}} OK done\r\n",
                    "1 ".repeat(count).trim_end()
                ),
            )],
            None,
        );
        assert!(s.uid_search("ALL").is_err());
        assert!(
            s.noop().is_err(),
            "cannot reset the poison with a new command"
        );
    }
}
#[test]
fn maximum_search_is_accepted_and_each_command_gets_a_fresh_budget() {
    let list = (1..=MAX_SEARCH_UIDS)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let reply = format!("* SEARCH {list}\r\n{{tag}} OK search\r\n");
    let (mut s, _, _) = session(
        vec![step("UID SEARCH", &reply), step("UID SEARCH", &reply)],
        None,
    );
    assert_eq!(s.uid_search("ALL").unwrap().len(), MAX_SEARCH_UIDS);
    assert_eq!(s.uid_search("ALL").unwrap().len(), MAX_SEARCH_UIDS);
}
#[test]
fn status_text_cannot_disguise_a_literal_to_bypass_search_limits() {
    let response = format!(
        "* OK text {{16000000}}\r\n* SEARCH {}\r\n{{tag}} OK search\r\n",
        "1 ".repeat(MAX_SEARCH_UIDS + 1)
    );
    let (mut s, _, _) = session(vec![step("UID SEARCH", &response)], None);
    assert!(s.uid_search("ALL").is_err());
}
#[test]
fn aggregate_response_and_protocol_line_counts_are_bounded() {
    let mut budget = Budget::default();
    for _ in 0..MAX_LINES {
        budget.receive(b"* 1 EXISTS\r\n").unwrap();
    }
    assert!(budget.receive(b"* 1 EXISTS\r\n").is_err());
    let mut budget = Budget::default();
    let chunk = [b'x'; 8192];
    for _ in 0..3 {
        budget.receive(b"* 1 FETCH (BODY[] {8388608}\r\n").unwrap();
        for _ in 0..1024 {
            budget.receive(&chunk).unwrap();
        }
        budget.receive(b")\r\n").unwrap();
    }
    assert!(budget.receive(b"* 1 FETCH (BODY[] {8388608}\r\n").is_err());
}
#[test]
fn response_literal_for_list_remains_supported() {
    let (mut s, _, _) = session(
        vec![step(
            "LIST",
            "* LIST () \"/\" {5}\r\nINBOX\r\n{tag} OK list\r\n",
        )],
        None,
    );
    assert_eq!(s.list(Some(""), Some("*")).unwrap()[0].name(), "INBOX");
}
