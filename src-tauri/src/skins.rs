use serde::Serialize;
use serde_json::Value;
use std::{fs, io::Read, path::Path};

const MAX_FILE_BYTES: u64 = 128 * 1024;
const MAX_SKINS: usize = 128;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSkin {
    folder_name: String,
    manifest: Value,
    tokens: Value,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinCatalog {
    pub skins: Vec<RawSkin>,
    pub warnings: Vec<String>,
}

fn read_json(root: &Path, file: &Path) -> Result<Value, String> {
    let resolved = fs::canonicalize(file).map_err(|_| "缺少皮肤文件".to_string())?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err("皮肤文件必须位于皮肤目录内".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(resolved)
        .map_err(|_| "无法读取皮肤文件".to_string())?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取皮肤文件".to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("皮肤 JSON 文件不能超过 128 KiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "皮肤 JSON 格式无效".into())
}

pub fn scan(root: &Path) -> Result<SkinCatalog, String> {
    fs::create_dir_all(root).map_err(|_| "无法创建皮肤目录".to_string())?;
    let root = fs::canonicalize(root).map_err(|_| "无法读取皮肤目录".to_string())?;
    let mut catalog = SkinCatalog::default();
    let mut entries = fs::read_dir(&root)
        .map_err(|_| "无法扫描皮肤目录".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "无法读取皮肤条目".to_string())?;
    entries.sort_by_key(|e| e.file_name());
    let directories: Vec<_> = entries
        .into_iter()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .collect();
    if directories.len() > MAX_SKINS {
        catalog
            .warnings
            .push("皮肤数量超过 128，仅加载前 128 个目录。".into());
    }
    for entry in directories.into_iter().take(MAX_SKINS) {
        let folder_name = entry.file_name().to_string_lossy().into_owned();
        // Resolve each pack separately: files cannot escape into a sibling pack either.
        let result = (|| {
            let pack =
                fs::canonicalize(entry.path()).map_err(|_| "无法读取皮肤目录".to_string())?;
            if !pack.starts_with(&root) {
                return Err("皮肤目录越界".to_string());
            }
            let manifest = read_json(&pack, &pack.join("manifest.json"))?;
            let tokens = read_json(&pack, &pack.join("tokens.json"))?;
            Ok(RawSkin {
                folder_name: folder_name.clone(),
                manifest,
                tokens,
            })
        })();
        match result {
            Ok(skin) => catalog.skins.push(skin),
            Err(error) => catalog.warnings.push(format!("{folder_name}: {error}")),
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_pack_does_not_hide_valid_packs() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["valid", "broken"] {
            fs::create_dir(directory.path().join(name)).unwrap();
        }
        fs::write(directory.path().join("valid/manifest.json"), "{}").unwrap();
        fs::write(directory.path().join("valid/tokens.json"), "{}").unwrap();
        let result = scan(directory.path()).unwrap();
        assert_eq!(result.skins.len(), 1);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn oversized_json_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("large.json");
        fs::write(&file, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        assert!(read_json(&root, &file).unwrap_err().contains("128 KiB"));
    }

    #[test]
    fn reading_outside_pack_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let pack = directory.path().join("pack");
        fs::create_dir(&pack).unwrap();
        let other = directory.path().join("other.json");
        fs::write(&other, "{}").unwrap();
        assert!(read_json(&fs::canonicalize(pack).unwrap(), &other).is_err());
    }
}
