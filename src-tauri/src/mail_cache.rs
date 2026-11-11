//! Bounded raw-message cache. Paths are derived from verified hashes, never mail filenames.
use crate::error::AppError;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
pub(crate) const MAX_RAW: u32 = 64 * 1024 * 1024;
const QUOTA: u64 = 512 * 1024 * 1024;
static CACHE_LOCK: Mutex<()> = Mutex::new(());
pub(crate) struct Spool {
    file: tempfile::NamedTempFile,
    root: PathBuf,
    expected: u32,
    written: usize,
    digest: Sha256,
}
fn storage() -> AppError {
    AppError::storage("邮件原文缓存不可用或磁盘空间不足")
}
fn key_valid(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(crate) fn root(conn: &rusqlite::Connection) -> Result<PathBuf, AppError> {
    let path: String = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| storage())?;
    if path.is_empty() {
        return Ok(std::env::temp_dir().join(format!("auumail-fixture-{}", uuid::Uuid::new_v4())));
    }
    Ok(Path::new(&path)
        .parent()
        .ok_or_else(storage)?
        .join("mail-cache"))
}
impl Spool {
    pub(crate) fn new(root: &Path, size: u32) -> Result<Self, AppError> {
        Self::reserve(root, size, QUOTA)
    }
    fn reserve(root: &Path, size: u32, quota: u64) -> Result<Self, AppError> {
        if size == 0 || size > MAX_RAW {
            return Err(AppError::invalid(
                "原文超过 64 MiB 下载上限，未跳过邮件或推进游标",
            ));
        }
        let _guard = CACHE_LOCK.lock().map_err(|_| storage())?;
        fs::create_dir_all(root).map_err(|_| storage())?;
        let mut used = 0;
        let mut candidates = vec![];
        for entry in fs::read_dir(root).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let meta = entry.path().symlink_metadata().map_err(|_| storage())?;
            if !meta.is_file() || meta.file_type().is_symlink() {
                continue;
            }
            used += meta.len();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.strip_suffix(".eml").is_some_and(key_valid) {
                candidates.push((meta.modified().ok(), entry.path(), meta.len()));
            }
        }
        candidates.sort_by_key(|entry| entry.0);
        for (_, path, size_on_disk) in candidates {
            if used + u64::from(size) <= quota {
                break;
            }
            fs::remove_file(path).map_err(|_| storage())?;
            used = used.saturating_sub(size_on_disk);
        }
        if used + u64::from(size) > quota {
            return Err(AppError::storage(
                "原文缓存配额不足，请等待其他下载完成后重试",
            ));
        }
        let mut file = tempfile::Builder::new()
            .prefix(".partial-")
            .tempfile_in(root)
            .map_err(|_| storage())?;
        file.as_file()
            .set_len(u64::from(size))
            .map_err(|_| storage())?;
        file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
        Ok(Self {
            file,
            root: root.into(),
            expected: size,
            written: 0,
            digest: Sha256::new(),
        })
    }
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), AppError> {
        if self.written + bytes.len() > self.expected as usize {
            return Err(AppError::network("原文分块超过声明大小，未保存"));
        }
        self.file.write_all(bytes).map_err(|_| storage())?;
        self.digest.update(bytes);
        self.written += bytes.len();
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<(String, Vec<u8>), AppError> {
        if self.written != self.expected as usize {
            return Err(AppError::network("原文分块不完整，未保存"));
        }
        self.file
            .flush()
            .and_then(|_| self.file.as_file().sync_data())
            .map_err(|_| storage())?;
        let key = format!("{:x}", self.digest.finalize());
        self.file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
        let mut raw = Vec::with_capacity(self.expected as usize);
        self.file.read_to_end(&mut raw).map_err(|_| storage())?;
        let _guard = CACHE_LOCK.lock().map_err(|_| storage())?;
        let destination = self.root.join(format!("{key}.eml"));
        if destination.exists() {
            let meta = destination.symlink_metadata().map_err(|_| storage())?;
            let mut valid = false;
            if meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() == u64::from(self.expected)
            {
                let mut existing = File::open(&destination).map_err(|_| storage())?;
                let mut digest = Sha256::new();
                let mut buffer = [0; 8192];
                loop {
                    let n = existing.read(&mut buffer).map_err(|_| storage())?;
                    if n == 0 {
                        break;
                    }
                    digest.update(&buffer[..n]);
                }
                valid = format!("{:x}", digest.finalize()) == key;
            }
            if valid {
                return Ok((key, raw));
            }
            fs::remove_file(&destination).map_err(|_| storage())?;
        }
        self.file.persist(destination).map_err(|_| storage())?;
        Ok((key, raw))
    }
}
pub(crate) fn recover(root: &Path) -> Result<(), AppError> {
    fs::create_dir_all(root).map_err(|_| storage())?;
    for entry in fs::read_dir(root).map_err(|_| storage())? {
        let entry = entry.map_err(|_| storage())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".partial-") && entry.file_type().map_err(|_| storage())?.is_file() {
            fs::remove_file(entry.path()).map_err(|_| storage())?;
        }
    }
    Ok(())
}
pub(crate) fn read(root: &Path, key: &str) -> Result<Vec<u8>, AppError> {
    if !key_valid(key) {
        return Err(AppError::invalid("原文缓存标识无效"));
    }
    let _guard = CACHE_LOCK.lock().map_err(|_| storage())?;
    let path = root.join(format!("{key}.eml"));
    let meta = path
        .symlink_metadata()
        .map_err(|_| AppError::invalid("原文缓存已释放，请重新下载原文"))?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > u64::from(MAX_RAW) {
        return Err(storage());
    }
    let mut file = File::open(path).map_err(|_| storage())?;
    let mut raw = vec![];
    (&mut file)
        .take(u64::from(MAX_RAW) + 1)
        .read_to_end(&mut raw)
        .map_err(|_| storage())?;
    if crate::mail_recovery::hash(&raw) != key {
        return Err(AppError::storage("原文缓存校验失败，请重新下载"));
    }
    let _ = file.set_modified(std::time::SystemTime::now());
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_reserves_inflight_bytes_and_releases_abandoned_spools_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let active = Spool::reserve(dir.path(), 6, 10).unwrap();
        assert!(Spool::reserve(dir.path(), 6, 10).is_err());
        drop(active);
        let mut complete = Spool::reserve(dir.path(), 6, 10).unwrap();
        complete.append(b"cached").unwrap();
        let (key, _) = complete.finish().unwrap();
        let active = Spool::reserve(dir.path(), 6, 10).unwrap();
        assert!(read(dir.path(), &key).is_err());
        drop(active);
        fs::write(dir.path().join(".partial-abandoned"), b"interrupted").unwrap();
        recover(dir.path()).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn incomplete_spools_are_removed_and_only_complete_hashed_raw_can_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut spool = Spool::new(dir.path(), 5).unwrap();
        spool.append(b"part").unwrap();
        assert!(spool.finish().is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        let mut spool = Spool::new(dir.path(), 5).unwrap();
        spool.append(b"hello").unwrap();
        let (key, raw) = spool.finish().unwrap();
        assert_eq!(read(dir.path(), &key).unwrap(), raw);
        fs::write(dir.path().join(format!("{key}.eml")), "wrong").unwrap();
        assert!(read(dir.path(), &key).is_err());
        assert!(read(dir.path(), "../secret").is_err());
    }
}
