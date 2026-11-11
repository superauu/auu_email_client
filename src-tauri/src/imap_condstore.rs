//! Optional CONDSTORE acceleration. Missing / regressed evidence falls back to full FLAGS.
use crate::error::AppError;
use imap_proto::{
    types::{AttributeValue, MailboxDatum, ResponseCode},
    Response,
};
use std::io::{Read, Write};
pub(crate) type FlagObservations = Vec<(u32, bool, bool)>;
fn invalid() -> AppError {
    AppError::network("CONDSTORE 响应不完整或超出请求，未采用增量水位")
}
fn responses(mut raw: &[u8]) -> Result<Vec<Response<'_>>, AppError> {
    let mut out = vec![];
    while !raw.is_empty() {
        let (rest, response) = imap_proto::parse_response(raw).map_err(|_| invalid())?;
        if rest.len() >= raw.len() {
            return Err(invalid());
        }
        raw = rest;
        out.push(response);
    }
    Ok(out)
}
pub(crate) fn select<S: Read + Write>(
    session: &mut imap::Session<S>,
    folder: &str,
    read_only: bool,
) -> Result<(imap::types::Mailbox, Option<u64>), AppError> {
    if folder.len() > 4096 || folder.chars().any(char::is_control) {
        return Err(AppError::invalid("文件夹名称不适合安全的 CONDSTORE 请求"));
    }
    let quoted = folder.replace('\\', "\\\\").replace('"', "\\\"");
    let command = format!(
        "{} \"{quoted}\" (CONDSTORE)",
        if read_only { "EXAMINE" } else { "SELECT" }
    );
    let raw = match session.run_command_and_read_response(command) {
        Ok(raw) => raw,
        Err(imap::Error::Bad(_) | imap::Error::No(_)) => {
            let mailbox = if read_only {
                session.examine(folder)
            } else {
                session.select(folder)
            }
            .map_err(|e| crate::imap_limits::network(e, "打开文件夹失败"))?;
            return Ok((mailbox, None));
        }
        Err(error) => {
            return Err(crate::imap_limits::network(
                error,
                "CONDSTORE 文件夹握手失败",
            ))
        }
    };
    parse_mailbox(&raw)
}
fn parse_mailbox(raw: &[u8]) -> Result<(imap::types::Mailbox, Option<u64>), AppError> {
    let mut mailbox = imap::types::Mailbox::default();
    let mut exists = None;
    let mut highest = None;
    for response in responses(raw)? {
        match response {
            Response::MailboxData(MailboxDatum::Exists(n)) => exists = Some(n),
            Response::Data {
                code: Some(code), ..
            } => match code {
                ResponseCode::UidValidity(n) => mailbox.uid_validity = Some(n),
                ResponseCode::UidNext(n) => mailbox.uid_next = Some(n),
                ResponseCode::HighestModSeq(n) => highest = (n > 0).then_some(n),
                _ => {}
            },
            _ => {}
        }
    }
    mailbox.exists = exists.ok_or_else(invalid)?;
    if mailbox.uid_validity.is_none_or(|n| n == 0) || mailbox.uid_next.is_none_or(|n| n == 0) {
        return Err(invalid());
    }
    Ok((mailbox, highest))
}
pub(crate) fn changed<S: Read + Write>(
    session: &mut imap::Session<S>,
    uids: &[u32],
    previous: u64,
) -> Result<Option<FlagObservations>, AppError> {
    let set = uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let command = format!("UID FETCH {set} (UID FLAGS MODSEQ) (CHANGEDSINCE {previous})");
    let raw = match session.run_command_and_read_response(command) {
        Ok(raw) => raw,
        Err(imap::Error::Bad(_) | imap::Error::No(_)) => return Ok(None),
        Err(error) => return Err(crate::imap_limits::network(error, "读取标记增量失败")),
    };
    parse_flags(&raw, uids, previous).map(Some)
}
fn parse_flags(
    raw: &[u8],
    uids: &[u32],
    previous: u64,
) -> Result<Vec<(u32, bool, bool)>, AppError> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for response in responses(raw)? {
        if let Response::Fetch(_, attrs) = response {
            let mut uid = None;
            let mut flags = None;
            let mut seq = None;
            for attr in attrs {
                match attr {
                    AttributeValue::Uid(n) if uid.is_none() => uid = Some(n),
                    AttributeValue::Flags(f) if flags.is_none() => flags = Some(f),
                    AttributeValue::ModSeq(n) if seq.is_none() => seq = Some(n),
                    _ => return Err(invalid()),
                }
            }
            let uid = uid
                .filter(|u| uids.contains(u) && seen.insert(*u))
                .ok_or_else(invalid)?;
            let flags = flags.ok_or_else(invalid)?;
            if seq.is_none_or(|n| n <= previous) {
                return Err(invalid());
            }
            out.push((
                uid,
                !flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")),
                flags.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged")),
            ));
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modseq_and_partial_flags_are_validated_without_treating_an_empty_delta_as_absence() {
        assert!(parse_flags(b"", &[1], 5).unwrap().is_empty());
        assert_eq!(
            parse_flags(b"* 1 FETCH (UID 1 FLAGS (\\Seen) MODSEQ (6))\r\n", &[1], 5).unwrap(),
            vec![(1, false, false)]
        );
        for raw in [
            "* 1 FETCH (UID 2 FLAGS () MODSEQ (6))\r\n",
            "* 1 FETCH (UID 1 FLAGS () MODSEQ (5))\r\n",
            "* 1 FETCH (UID 1 FLAGS ())\r\n",
        ] {
            assert!(parse_flags(raw.as_bytes(), &[1], 5).is_err());
        }
        assert!(parse_mailbox(b"* 1 EXISTS\r\n* OK [HIGHESTMODSEQ 100] value\r\n").is_err());
        let(m,h)=parse_mailbox(b"* 1 EXISTS\r\n* OK [UIDVALIDITY 2] value\r\n* OK [UIDNEXT 3] value\r\n* OK [HIGHESTMODSEQ 100] value\r\n").unwrap();
        assert_eq!((m.uid_validity, h), (Some(2), Some(100)));
    }
}
