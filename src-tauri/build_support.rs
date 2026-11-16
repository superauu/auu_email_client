use std::{ffi::OsString, path::Path};

/// An explicitly empty environment value disables the local fallback (useful in CI).
pub fn load_secret(env: Option<OsString>, local: &Path) -> Result<Option<String>, &'static str> {
    let value = match env {
        Some(value) => value
            .into_string()
            .map_err(|_| "OAuth build secret must be UTF-8")?,
        None => match std::fs::read_to_string(local) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Could not read the local OAuth build configuration"),
        },
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 4096 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(
            "OAuth build secret must be a single printable ASCII value (at most 4096 bytes)",
        );
    }
    Ok(Some(value.to_owned()))
}

/// Rust-escape the value; only write it into ignored Cargo build output, never logs.
pub fn secret_source(secret: Option<&str>) -> String {
    format!("const GOOGLE_BUILTIN_CLIENT_SECRET: Option<&str> = {secret:?};\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_overrides_local_and_empty_environment_disables_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.local");
        std::fs::write(&path, "synthetic-local\n").unwrap();
        assert_eq!(
            load_secret(None, &path).unwrap().as_deref(),
            Some("synthetic-local")
        );
        assert_eq!(
            load_secret(Some("synthetic-env".into()), &path)
                .unwrap()
                .as_deref(),
            Some("synthetic-env")
        );
        assert_eq!(load_secret(Some("".into()), &path).unwrap(), None);
    }

    #[test]
    fn missing_secret_is_optional_and_invalid_values_never_appear_in_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.local");
        assert_eq!(load_secret(None, &path).unwrap(), None);
        let error = load_secret(Some("synthetic\ninjected".into()), &path).unwrap_err();
        assert!(!error.contains("synthetic"));
        assert!(load_secret(Some("x".repeat(4097).into()), &path).is_err());
        assert_eq!(
            secret_source(None),
            "const GOOGLE_BUILTIN_CLIENT_SECRET: Option<&str> = None;\n"
        );
        assert!(secret_source(Some("synthetic\"\\value")).contains("synthetic\\\"\\\\value"));
    }
}
