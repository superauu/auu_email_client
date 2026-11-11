//! Bounded UID-range collection followed by a separate full-range membership validation pass.
use super::*;
pub(super) const SPAN: u32 = 50_000;
const TARGET_CANDIDATES: u32 = 1024;
pub(super) fn should_segment(mailbox: &imap::types::Mailbox, header_hint: bool) -> bool {
    mailbox.uid_next.is_some_and(|next| {
        next > 0 && (next > SPAN + 1 || (!header_hint && mailbox.exists > TARGET_CANDIDATES))
    })
}
fn default_span() -> u32 {
    SPAN
}
fn is_default_span(span: &u32) -> bool {
    *span == SPAN
}
#[derive(Clone, Serialize, Deserialize)]
struct Fence {
    name: String,
    kind: String,
    validity: u32,
    next: u32,
    // Omitted for v1 checkpoints / sparse mailboxes: their initial hash stays compatible.
    #[serde(default = "default_span", skip_serializing_if = "is_default_span")]
    span: u32,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Fixed {
    location: Location,
    hash: Option<String>,
    header: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Current {
    ordinal: usize,
    uids: Vec<u32>,
    offset: usize,
}
#[derive(Serialize, Deserialize)]
struct Found {
    location: Location,
    hash: String,
    header: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Checkpoint {
    #[serde(default, skip_serializing_if = "is_false")]
    wide: bool,
    format: String,
    fences: Vec<Fence>,
    fixed: Option<Fixed>,
    cross: bool,
    collected: usize,
    validated: usize,
    current: Option<Current>,
    checked: usize,
    discovered: usize,
    collected_root: String,
    validated_root: String,
    found: Vec<Found>,
}
fn is_false(value: &bool) -> bool {
    !*value
}
pub(super) fn is_checkpoint(json: &str) -> bool {
    #[derive(Deserialize)]
    struct Format {
        #[serde(default)]
        format: Option<String>,
    }
    // Unknown fields are skipped without materializing another 50000-element Value tree.
    serde_json::from_str::<Format>(json).is_ok_and(|v| v.format.as_deref() == Some("segmented-v1"))
}
fn count(f: &Fence) -> usize {
    (u64::from(f.next - 1).div_ceil(u64::from(f.span))) as usize
}
impl Checkpoint {
    fn total_segments(&self) -> usize {
        self.fences.iter().map(count).sum()
    }
    fn range(&self, mut ordinal: usize) -> Result<(usize, u32, u32), AppError> {
        for (index, f) in self.fences.iter().enumerate() {
            let n = count(f);
            if ordinal < n {
                let start = ordinal as u64 * u64::from(f.span) + 1;
                let end = (start + u64::from(f.span) - 1).min(u64::from(f.next - 1));
                return Ok((index, start as u32, end as u32));
            }
            ordinal -= n;
        }
        Err(unresolved("分段游标超出快照范围。"))
    }
    pub(super) fn progress(&self) -> Progress {
        let total = self.total_segments();
        Progress {
            checked: self.checked,
            total: self.discovered,
            matches: self.found.len()
                + usize::from(self.fixed.as_ref().is_some_and(|f| f.hash.is_some())),
            cross_uid_validity: self.cross,
            segments: Some(SegmentProgress {
                completed: if self.collected < total {
                    self.collected
                } else {
                    self.validated
                },
                total,
                phase: if self.collected < total {
                    "collecting"
                } else {
                    "validating"
                }
                .into(),
            }),
        }
    }
}
fn fence<S: Read + Write>(
    session: &mut imap::Session<S>,
    name: &str,
    kind: &str,
) -> Result<Fence, AppError> {
    let mailbox = session
        .examine(name)
        .map_err(|error| crate::imap_limits::network(error, "无法读取分段文件夹边界"))?;
    let next = mailbox
        .uid_next
        .filter(|n| *n > 0)
        .ok_or_else(|| unresolved("分段核对缺少 UIDNEXT，不能证明完整范围。"))?;
    // Estimate density only to select a fixed interval width. Never use EXISTS as
    // completeness evidence, nor change a saved width when later EXISTS changes.
    let span = (u64::from(next - 1) * u64::from(TARGET_CANDIDATES))
        .div_ceil(u64::from(mailbox.exists.max(1)))
        .clamp(u64::from(TARGET_CANDIDATES), u64::from(SPAN)) as u32;
    Ok(Fence {
        name: name.into(),
        kind: kind.into(),
        validity: mailbox
            .uid_validity
            .filter(|n| *n > 0)
            .ok_or_else(|| unresolved("分段核对缺少 UIDVALIDITY。"))?,
        next,
        span,
    })
}
fn fixed<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    total: &mut usize,
) -> Result<Fixed, AppError> {
    if examine(session, &s.source.folder)? != s.source.uid_validity {
        return Err(unresolved("分段期间源 UIDVALIDITY 变化。"));
    }
    let mail = fetch(session, s.source.uid, &s.cached, total)?;
    if mail.as_ref().is_some_and(|mail| !mail.matches) {
        return Err(unresolved("源邮件内容不同，不能核对移动结果。"));
    }
    Ok(Fixed {
        location: s.source.clone(),
        hash: mail.as_ref().map(|m| m.hash.clone()),
        header: mail.and_then(|m| m.header_id),
    })
}
fn validate_fences<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    c: &Checkpoint,
    total: &mut usize,
) -> Result<(), AppError> {
    for f in &c.fences {
        let now = fence(session, &f.name, &f.kind)?;
        if now.validity != f.validity || now.next != f.next {
            return Err(unresolved(
                "分段 UIDVALIDITY / UIDNEXT 边界已变化，证据失效，请重新核对。",
            ));
        }
    }
    if let Some(saved) = &c.fixed {
        if fixed(session, s, total)?.ne(saved) {
            return Err(unresolved("分段期间源存在性或内容改变。"));
        }
    }
    Ok(())
}
fn uids<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    c: &Checkpoint,
    ordinal: usize,
) -> Result<Vec<u32>, AppError> {
    let (index, start, end) = c.range(ordinal)?;
    let f = &c.fences[index];
    let now = fence(session, &f.name, &f.kind)?;
    if now.validity != f.validity || now.next != f.next {
        return Err(unresolved("读取分段前命名空间边界变化。"));
    }
    let result = session
        .uid_search(format!("UID {start}:{end} {}", scan::query(s)?))
        .map_err(|error| crate::imap_limits::network(error, "分段 UID 搜索失败"))?;
    if result.len() > f.span as usize || result.iter().any(|uid| *uid < start || *uid > end) {
        return Err(unresolved("分段搜索含越界或超量 UID，未采用部分结果。"));
    }
    let mut result: Vec<_> = result.into_iter().collect();
    result.sort_unstable();
    Ok(result)
}
fn fold(root: &str, ordinal: usize, uids: &[u32]) -> Result<String, AppError> {
    serde_json::to_vec(&(root, ordinal, uids))
        .map(|bytes| hash(&bytes))
        .map_err(|_| AppError::storage("无法计算分段集合摘要"))
}
fn initialize<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    total: &mut usize,
    names: &[String],
) -> Result<Checkpoint, AppError> {
    let wide = s.cached.remote_missing && s.archive.is_none();
    if wide && (!trusted_hash(&s.cached) || names.len() > 128) {
        return Err(unresolved(
            "跨文件夹核对需要可信完整原文，且可访问文件夹不能超过 128 个。",
        ));
    }
    let mut source = fence(session, &s.source.folder, &s.source.kind)?;
    // EXISTS estimates ALL density only. A selective HEADER search must not
    // acquire thousands of empty intervals solely because the mailbox is dense.
    if s.cached.header_message_id.is_some() {
        source.span = SPAN;
    }
    let cross = source.validity != s.source.uid_validity;
    if cross && !trusted_hash(&s.cached) {
        return Err(unresolved("跨代分段绑定需要可信的完整原文指纹。"));
    }
    let mut fences = vec![];
    let fixed = if cross || s.archive.is_none() {
        fences.push(source);
        None
    } else {
        Some(fixed(session, s, total)?)
    };
    if let Some((target, _)) = &s.archive {
        if target == &s.source.folder {
            return Err(unresolved("源与目标相同。"));
        }
        let mut target = fence(session, target, "archive")?;
        if s.cached.header_message_id.is_some() {
            target.span = SPAN;
        }
        fences.push(target);
    }
    if wide {
        let mut others: Vec<_> = names
            .iter()
            .filter(|name| *name != &s.source.folder)
            .collect();
        others.sort();
        others.dedup();
        for name in others {
            let mut target = fence(session, name, &crate::imap_settings::scope(name))?;
            if s.cached.header_message_id.is_some() {
                target.span = SPAN
            }
            fences.push(target);
        }
    }
    let initial = hash(
        &serde_json::to_vec(&(&fences, &fixed))
            .map_err(|_| AppError::storage("无法计算分段边界摘要"))?,
    );
    Ok(Checkpoint {
        wide,
        format: "segmented-v1".into(),
        fences,
        fixed,
        cross,
        collected: 0,
        validated: 0,
        current: None,
        checked: 0,
        discovered: 0,
        collected_root: initial.clone(),
        validated_root: initial,
        found: vec![],
    })
}
pub(super) fn page<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    names: &[String],
) -> Result<Inspection, AppError> {
    if !names.contains(&s.source.folder)
        || s.archive
            .as_ref()
            .is_some_and(|(target, _)| !names.contains(target))
    {
        return Err(unresolved("分段核对文件夹缺失。"));
    }
    let mut total = 0;
    let mut c = if let Some(json) = &s.scan_json {
        let c: Checkpoint = serde_json::from_str(json)
            .map_err(|_| AppError::storage("分段检查点无效，请重新核对"))?;
        if c.format != "segmented-v1" {
            return Err(unresolved("分段检查点版本无效。"));
        }
        if c.fences.is_empty()
            || c.fences.len() > if c.wide { 128 } else { 2 }
            || c.fences.iter().any(|f| {
                f.validity == 0 || f.next == 0 || !(TARGET_CANDIDATES..=SPAN).contains(&f.span)
            })
        {
            return Err(unresolved("分段检查点边界无效。"));
        }
        if (c.cross || c.wide) && !trusted_hash(&s.cached) {
            return Err(unresolved("分段检查点缺少可信历史指纹。"));
        }
        validate_fences(session, s, &c, &mut total)?;
        c
    } else {
        initialize(session, s, &mut total, names)?
    };
    if c.wide {
        let mut expected: Vec<_> = c.fences.iter().map(|f| f.name.as_str()).collect();
        expected.sort();
        let mut actual: Vec<_> = names.iter().map(String::as_str).collect();
        actual.sort();
        actual.dedup();
        if expected != actual {
            return Err(unresolved(
                "跨文件夹核对期间可访问目录集合改变，请重新核对。",
            ));
        }
    }
    let segments = c.total_segments();
    if c.collected > segments || c.validated > c.collected {
        return Err(unresolved("分段检查点游标无效。"));
    }
    if c.collected < segments {
        let candidates = uids(session, s, &c, c.collected)?;
        if let Some(current) = &c.current {
            if current.ordinal != c.collected || current.uids != candidates {
                return Err(unresolved("当前分段候选集合已改变。"));
            }
        } else {
            c.discovered += candidates.len();
            c.current = Some(Current {
                ordinal: c.collected,
                uids: candidates,
                offset: 0,
            });
        }
        let current = c.current.as_ref().expect("initialized segment");
        let candidates = &current.uids;
        if current.offset > candidates.len() {
            return Err(unresolved("分段页游标无效。"));
        }
        let begin = current.offset;
        let end = (begin + scan::PAGE_SIZE).min(candidates.len());
        let (index, _, _) = c.range(c.collected)?;
        let f = &c.fences[index];
        for uid in &candidates[begin..end] {
            let mail = fetch(session, *uid, &s.cached, &mut total)?
                .ok_or_else(|| unresolved("分段候选消失。"))?;
            c.checked += 1;
            if mail.matches {
                c.found.push(Found {
                    location: Location {
                        folder: f.name.clone(),
                        kind: f.kind.clone(),
                        uid_validity: f.validity,
                        uid: *uid,
                    },
                    hash: mail.hash,
                    header: mail.header_id,
                });
            }
            if c.progress().matches > 1 {
                return Err(unresolved("不同分段出现重复匹配或源目标均存在，未绑定。"));
            }
        }
        if uids(session, s, &c, c.collected)?.as_slice() != candidates.as_slice() {
            return Err(unresolved("读取页面后当前分段候选变化。"));
        }
        validate_fences(session, s, &c, &mut total)?;
        if end == candidates.len() {
            c.collected_root = fold(&c.collected_root, c.collected, candidates)?;
            c.collected += 1;
            c.current = None;
        } else {
            c.current.as_mut().expect("active segment").offset = end;
        }
        return Ok(Inspection::Segment(c));
    }
    if c.validated < segments {
        let candidates = uids(session, s, &c, c.validated)?;
        c.validated_root = fold(&c.validated_root, c.validated, &candidates)?;
        c.validated += 1;
        validate_fences(session, s, &c, &mut total)?;
        if c.validated < segments {
            return Ok(Inspection::Segment(c));
        }
    }
    if c.validated_root != c.collected_root {
        return Err(unresolved("全段复核发现候选集合变化，分段证据失效。"));
    }
    let (location, digest, header, result, target) = if let Some(found) = c.found.first() {
        if examine(session, &found.location.folder)? != found.location.uid_validity {
            return Err(unresolved("最终匹配命名空间已改变。"));
        }
        let mail = fetch(session, found.location.uid, &s.cached, &mut total)?
            .filter(|m| m.matches && m.hash == found.hash)
            .ok_or_else(|| unresolved("最终原文复核失败。"))?;
        let result = if s.archive.is_none() {
            "legacy"
        } else if found.location.folder == s.source.folder {
            "source-present"
        } else {
            "archived"
        };
        let target = if result == "source-present" {
            c.fences
                .iter()
                .find(|f| f.name != s.source.folder)
                .map(|f| (f.name.clone(), f.validity))
        } else {
            None
        };
        (
            found.location.clone(),
            mail.hash,
            mail.header_id,
            result,
            target,
        )
    } else if let Some(source) = c.fixed.as_ref().filter(|f| f.hash.is_some()) {
        let target = c
            .fences
            .first()
            .ok_or_else(|| unresolved("缺少目标分段。"))?;
        (
            source.location.clone(),
            source.hash.clone().expect("checked source"),
            source.header.clone(),
            "source-present",
            Some((target.name.clone(), target.validity)),
        )
    } else {
        return Err(unresolved("完成所有分段与全段复核后没有匹配。"));
    };
    validate_fences(session, s, &c, &mut total)?;
    Ok(Inspection::Complete(Proof {
        location,
        hash: digest,
        header_id: header,
        result: result.into(),
        strength: strength(&s.cached),
        target,
        scan_guard: Some(c.collected_root),
        cross_uid_validity: c.cross,
    }))
}
