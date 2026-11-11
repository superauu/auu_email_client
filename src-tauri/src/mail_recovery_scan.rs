//! Persisted pages are observations, never proof of uniqueness until the entire snapshot is checked.
use super::*;
pub(super) const PAGE_SIZE: usize = 8;
const MAX_SNAPSHOT: usize = 50_000;
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Folder {
    name: String,
    kind: String,
    validity: u32,
    uids: Vec<u32>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FixedSource {
    location: Location,
    hash: Option<String>,
    header_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Match {
    folder: usize,
    uid: u32,
    hash: String,
    header_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Checkpoint {
    folders: Vec<Folder>,
    fixed: Option<FixedSource>,
    offset: usize,
    matches: Vec<Match>,
    cross: bool,
}
impl Checkpoint {
    pub(super) fn progress(&self) -> Progress {
        Progress {
            checked: self.offset,
            total: self.folders.iter().map(|f| f.uids.len()).sum(),
            matches: self.matches.len()
                + usize::from(self.fixed.as_ref().is_some_and(|s| s.hash.is_some())),
            cross_uid_validity: self.cross,
            segments: None,
        }
    }
}
pub(super) fn query(s: &Snapshot) -> Result<String, AppError> {
    if let Some(id) = &s.cached.header_message_id {
        if id.len() > 998 || id.chars().any(char::is_control) {
            return Err(unresolved("缓存 Message-ID 不适合安全搜索。"));
        }
        Ok(format!(
            "HEADER Message-ID \"{}\"",
            id.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    } else {
        Ok("ALL".into())
    }
}
fn folder<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    name: &str,
    kind: &str,
) -> Result<Folder, AppError> {
    let validity = examine(session, name)?;
    let uids = session
        .uid_search(query(s)?)
        .map_err(|error| crate::imap_limits::network(error, "分页候选搜索失败"))?;
    if uids.len() > MAX_SNAPSHOT || uids.contains(&0) {
        return Err(unresolved(
            "候选快照超过 50000 UID 或含无效 UID；未采用部分结果。",
        ));
    }
    let mut uids: Vec<_> = uids.into_iter().collect();
    uids.sort_unstable();
    Ok(Folder {
        name: name.into(),
        kind: kind.into(),
        validity,
        uids,
    })
}
fn fixed<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    total: &mut usize,
) -> Result<FixedSource, AppError> {
    if examine(session, &s.source.folder)? != s.source.uid_validity {
        return Err(unresolved(
            "分页核对期间源 UIDVALIDITY 已改变，请重新开始。",
        ));
    }
    let mail = fetch(session, s.source.uid, &s.cached, total)?;
    if mail.as_ref().is_some_and(|m| !m.matches) {
        return Err(unresolved("源 UID 内容不同，不能恢复。"));
    }
    Ok(FixedSource {
        location: s.source.clone(),
        hash: mail.as_ref().map(|m| m.hash.clone()),
        header_id: mail.and_then(|m| m.header_id),
    })
}
fn validate<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    c: &Checkpoint,
    total: &mut usize,
) -> Result<(), AppError> {
    for saved in &c.folders {
        if &folder(session, s, &saved.name, &saved.kind)? != saved {
            return Err(unresolved(
                "候选集合或 UIDVALIDITY 已改变，分页证据失效，请重新核对。",
            ));
        }
    }
    if let Some(source) = &c.fixed {
        if fixed(session, s, total)?.ne(source) {
            return Err(unresolved("分页期间源邮件存在性或内容变化，请重新核对。"));
        }
    }
    Ok(())
}
fn initialize<S: Read + Write>(
    session: &mut imap::Session<S>,
    s: &Snapshot,
    total: &mut usize,
) -> Result<Checkpoint, AppError> {
    let validity = examine(session, &s.source.folder)?;
    let cross = validity != s.source.uid_validity;
    if cross && !trusted_hash(&s.cached) {
        return Err(unresolved(
            "跨 UIDVALIDITY 重绑定需要已有完整原文 SHA-256，不能仅凭缓存字段或 Message-ID 绑定。",
        ));
    }
    let mut folders = vec![];
    let fixed = if cross || s.archive.is_none() {
        folders.push(folder(session, s, &s.source.folder, &s.source.kind)?);
        None
    } else {
        Some(fixed(session, s, total)?)
    };
    if let Some((target, _)) = &s.archive {
        if target == &s.source.folder {
            return Err(unresolved("源与归档目标相同，不能核对。"));
        }
        folders.push(folder(session, s, target, "archive")?);
    }
    if folders.iter().map(|f| f.uids.len()).sum::<usize>() > MAX_SNAPSHOT {
        return Err(unresolved("合计候选超过 50000 UID，未采用部分结果。"));
    }
    // The initial EXAMINE and the source search must agree as well.
    if folders
        .first()
        .is_some_and(|f| f.name == s.source.folder && f.validity != validity)
    {
        return Err(unresolved("建立快照时源 UIDVALIDITY 已改变。"));
    }
    Ok(Checkpoint {
        folders,
        fixed,
        offset: 0,
        matches: vec![],
        cross,
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
        return Err(unresolved("分页核对文件夹已不在当前服务器列表中。"));
    }
    let mut total = 0;
    let mut c = if let Some(json) = &s.scan_json {
        let c: Checkpoint = serde_json::from_str(json)
            .map_err(|_| AppError::storage("分页检查点损坏，请重新开始核对"))?;
        if c.cross && !trusted_hash(&s.cached) {
            return Err(unresolved("检查点缺少可信历史原文指纹，不能继续跨代绑定。"));
        }
        validate(session, s, &c, &mut total)?;
        c
    } else {
        initialize(session, s, &mut total)?
    };
    let count = c.progress().total;
    if c.offset > count {
        return Err(AppError::storage("分页游标无效，请重新核对"));
    }
    let mut skipped = 0;
    let mut page = vec![];
    for (index, f) in c.folders.iter().enumerate() {
        for uid in &f.uids {
            if skipped >= c.offset && page.len() < PAGE_SIZE {
                page.push((index, *uid));
            }
            skipped += 1;
        }
    }
    let mut selected = None;
    for (index, uid) in page {
        if selected != Some(index) {
            if examine(session, &c.folders[index].name)? != c.folders[index].validity {
                return Err(unresolved("读取分页前 UIDVALIDITY 改变。"));
            }
            selected = Some(index);
        }
        let mail = fetch(session, uid, &s.cached, &mut total)?
            .ok_or_else(|| unresolved("分页候选消失，请重新核对。"))?;
        c.offset += 1;
        if mail.matches {
            c.matches.push(Match {
                folder: index,
                uid,
                hash: mail.hash,
                header_id: mail.header_id,
            });
        }
        if c.progress().matches > 1 {
            return Err(unresolved(
                "已发现重复匹配或源与目标均存在，不选择第一个匹配。",
            ));
        }
    }
    validate(session, s, &c, &mut total)?;
    if c.offset < count {
        return Ok(Inspection::Page(c));
    }
    let (location, digest, header_id, result, target) = if let Some(found) = c.matches.first() {
        let f = &c.folders[found.folder];
        // Re-read the winning candidate after the final membership check.
        if examine(session, &f.name)? != f.validity {
            return Err(unresolved("完成扫描后 UIDVALIDITY 改变。"));
        }
        let mail = fetch(session, found.uid, &s.cached, &mut total)?
            .ok_or_else(|| unresolved("匹配候选已消失。"))?;
        if !mail.matches || mail.hash != found.hash {
            return Err(unresolved("匹配候选内容已改变。"));
        }
        let location = Location {
            folder: f.name.clone(),
            kind: f.kind.clone(),
            uid_validity: f.validity,
            uid: found.uid,
        };
        let in_source = f.name == s.source.folder;
        let result = if s.archive.is_none() {
            "legacy"
        } else if in_source {
            "source-present"
        } else {
            "archived"
        };
        let target = if result == "source-present" {
            c.folders
                .iter()
                .find(|f| f.kind == "archive" && f.name != s.source.folder)
                .map(|f| (f.name.clone(), f.validity))
        } else {
            None
        };
        (location, mail.hash, mail.header_id, result, target)
    } else if let Some(source) = c.fixed.as_ref().filter(|f| f.hash.is_some()) {
        let dest = c
            .folders
            .first()
            .ok_or_else(|| unresolved("缺少目标候选快照。"))?;
        (
            source.location.clone(),
            source.hash.clone().expect("filtered hash"),
            source.header_id.clone(),
            "source-present",
            Some((dest.name.clone(), dest.validity)),
        )
    } else {
        return Err(unresolved("完整分页核对后没有匹配，保留本机修改。"));
    };
    let bytes = serde_json::to_vec(&(&c.folders, &c.fixed))
        .map_err(|_| AppError::storage("无法保存候选集合摘要"))?;
    Ok(Inspection::Complete(Proof {
        location,
        hash: digest,
        header_id,
        result: result.into(),
        strength: strength(&s.cached),
        target,
        scan_guard: Some(hash(&bytes)),
        cross_uid_validity: c.cross,
    }))
}
