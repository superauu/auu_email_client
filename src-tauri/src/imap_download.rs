//! Size-planned FETCH and exact-offset partial BODY downloads; never skip an oversized UID.
use crate::{
    error::AppError,
    mail_cache::{Spool, MAX_RAW},
    mail_sync::{self, ParsedMessage},
};
use imap_proto::{types::AttributeValue, Response};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Read, Write},
    path::Path,
};
const BATCH_BYTES: u32 = 8 * 1024 * 1024;
const CHUNK_BYTES: u32 = 256 * 1024;
#[cfg(test)]
#[path = "imap_download_tests.rs"]
mod tests;
fn invalid() -> AppError {
    AppError::network("IMAP 下载响应的 UID、偏移或大小不一致，本批次未提交")
}
pub(crate) fn fetch_range<S: Read + Write>(
    session: &mut imap::Session<S>,
    start: u32,
    end: u32,
    folder: &str,
    validity: u32,
    cache: &Path,
) -> Result<Vec<ParsedMessage>, AppError> {
    if start == 0 || end < start || end - start >= 100 {
        return Err(AppError::invalid("下载 UID 区间无效或超过 100"));
    }
    let metadata = session
        .uid_fetch(
            format!("{start}:{end}"),
            "(UID RFC822.SIZE FLAGS INTERNALDATE)",
        )
        .map_err(|e| crate::imap_limits::network(e, "读取邮件下载索引失败"))?;
    let mut sizes = BTreeMap::new();
    for m in metadata.iter() {
        let uid = m
            .uid
            .filter(|uid| *uid >= start && *uid <= end)
            .ok_or_else(invalid)?;
        let size = m.size.filter(|size| *size > 0).ok_or_else(invalid)?;
        if sizes.insert(uid, size).is_some() || sizes.len() > 100 {
            return Err(invalid());
        }
        if size > MAX_RAW {
            return Err(AppError::invalid(
                "邮件原文超过 64 MiB 下载上限；缓存和游标保留，未跳过该邮件",
            ));
        }
    }
    drop(metadata);
    let mut output = vec![];
    let mut group = vec![];
    let mut bytes = 0;
    for (&uid, &size) in &sizes {
        if !group.is_empty() && (bytes + size > BATCH_BYTES || size > BATCH_BYTES) {
            fetch_group(session, &group, &sizes, cache, &mut output)?;
            group.clear();
            bytes = 0;
        }
        if size > BATCH_BYTES {
            output.push(partial(session, uid, size, cache)?);
        } else {
            group.push(uid);
            bytes += size;
        }
    }
    if !group.is_empty() {
        fetch_group(session, &group, &sizes, cache, &mut output)?;
    }
    // A successful body response must not be rebound to a reset UID namespace.
    let now = session
        .examine(folder)
        .map_err(|e| crate::imap_limits::network(e, "下载后复核文件夹失败"))?;
    if now.uid_validity != Some(validity) {
        return Err(AppError::conflict("下载期间 UIDVALIDITY 改变，未提交原文"));
    }
    Ok(output)
}
fn fetch_group<S: Read + Write>(
    s: &mut imap::Session<S>,
    group: &[u32],
    sizes: &BTreeMap<u32, u32>,
    cache: &Path,
    out: &mut Vec<ParsedMessage>,
) -> Result<(), AppError> {
    let set = group
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let fetched = s
        .uid_fetch(set, "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])")
        .map_err(|e| crate::imap_limits::network(e, "下载邮件原文失败"))?;
    let mut seen = HashSet::new();
    for m in fetched.iter() {
        let uid = m
            .uid
            .filter(|u| group.contains(u) && seen.insert(*u))
            .ok_or_else(invalid)?;
        let raw = m
            .body()
            .filter(|raw| raw.len() == sizes[&uid] as usize)
            .ok_or_else(invalid)?;
        if m.size.is_some_and(|size| size != sizes[&uid]) {
            return Err(invalid());
        }
        let mut spool = Spool::new(cache, sizes[&uid])?;
        spool.append(raw)?;
        let (key, _) = spool.finish()?;
        let mut parsed = mail_sync::parse_message(m)?;
        parsed.raw_cache_key = Some(key);
        out.push(parsed);
    }
    if seen.len() != group.len() {
        return Err(invalid());
    }
    Ok(())
}
fn partial<S: Read + Write>(
    s: &mut imap::Session<S>,
    uid: u32,
    size: u32,
    cache: &Path,
) -> Result<ParsedMessage, AppError> {
    let mut spool = Spool::new(cache, size)?;
    let mut offset = 0;
    while offset < size {
        let count = CHUNK_BYTES.min(size - offset);
        let response = s
            .run_command_and_read_response(format!(
                "UID FETCH {uid} (UID BODY.PEEK[]<{offset}.{count}>)"
            ))
            .map_err(|e| crate::imap_limits::network(e, "分块下载原文失败"))?;
        spool.append(chunk(&response, uid, offset, count)?)?;
        offset += count;
    }
    let metadata = s
        .uid_fetch(uid.to_string(), "(UID RFC822.SIZE FLAGS INTERNALDATE)")
        .map_err(|e| crate::imap_limits::network(e, "分块下载后复核大小失败"))?;
    if metadata.len() != 1 || metadata[0].uid != Some(uid) || metadata[0].size != Some(size) {
        return Err(invalid());
    }
    let m = &metadata[0];
    let (key, raw) = spool.finish()?;
    let mut parsed = mail_sync::parse_raw(
        uid,
        &raw,
        m.internal_date()
            .ok_or_else(invalid)?
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        !m.flags().contains(&imap::types::Flag::Seen),
        m.flags().contains(&imap::types::Flag::Flagged),
    )?;
    parsed.raw_cache_key = Some(key);
    Ok(parsed)
}
fn chunk(response: &[u8], uid: u32, offset: u32, count: u32) -> Result<&[u8], AppError> {
    let mut remaining = response;
    let mut body = None;
    let mut fetched = false;
    while !remaining.is_empty() {
        let (rest, parsed) = imap_proto::parse_response(remaining).map_err(|_| invalid())?;
        if rest.len() >= remaining.len() {
            return Err(invalid());
        }
        remaining = rest;
        if let Response::Fetch(_, attrs) = parsed {
            let mut ids = attrs.iter().filter_map(|attr| {
                if let AttributeValue::Uid(uid) = attr {
                    Some(*uid)
                } else {
                    None
                }
            });
            if fetched || ids.next() != Some(uid) || ids.next().is_some() {
                return Err(invalid());
            }
            fetched = true;
            for attr in attrs {
                if let AttributeValue::BodySection {
                    section: None,
                    index: Some(index),
                    data: Some(data),
                } = attr
                {
                    if index != offset
                        || data.len() != count as usize
                        || body.replace(data).is_some()
                    {
                        return Err(invalid());
                    }
                }
            }
        }
    }
    body.ok_or_else(invalid)
}
