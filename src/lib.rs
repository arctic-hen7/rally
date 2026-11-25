use anyhow::{Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

pub fn secret() -> String {
    use rand::RngCore;
    let mut value = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut value);
    hex::encode(value)
}

pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub fn field<'a>(data: &'a Value, key: &str) -> Result<&'a str> {
    data[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing {key}"))
}

pub fn opt<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or("")
}

pub fn slug(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 80
        || !value.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        || value.contains("..")
    {
        bail!("Use a short name containing letters, digits, hyphens, underscores or dots");
    }
    Ok(value)
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("RALLY_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
                })
                .join("rally")
        })
}

pub fn read_json(path: &Path) -> Result<Value> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}

pub fn write_json(path: &Path, data: &Value) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension(id("tmp"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(data)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub type GitRoutes = std::collections::BTreeMap<String, (String, bool)>;

pub fn rewrite_git_url(routes: &GitRoutes, original: &str) -> Option<String> {
    let original = original.trim_end_matches('/');
    let stem = original.strip_suffix(".git").unwrap_or(original);
    let (source, (target, prefix)) = routes
        .iter()
        .filter(|(source, (_, prefix))| {
            if *prefix {
                original.starts_with(source.as_str())
            } else {
                stem == source.as_str()
            }
        })
        .max_by_key(|(source, _)| source.len())?;
    Some(if *prefix {
        format!(
            "{}/{}",
            target.trim_end_matches('/'),
            &original[source.len()..]
        )
    } else {
        target.clone()
    })
}

pub fn remote(value: &str) -> Result<&str> {
    if value.starts_with('-') || value.contains("::") || value.chars().any(char::is_control) {
        bail!("Invalid Git remote");
    }
    if !(value.starts_with("https://")
        || value.starts_with("ssh://")
        || (value.starts_with("git@") && value.contains(':')))
    {
        bail!("Use an HTTPS or SSH Git remote");
    }
    Ok(value)
}

pub fn materialize_skills(root: &Path, bundle: &Value) -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::unix::fs::PermissionsExt;
    for (name, files) in bundle
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Invalid skill set"))?
    {
        let dir = root.join(slug(name)?);
        for (path, file) in files
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Invalid skill bundle"))?
        {
            if !Path::new(path)
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_)))
            {
                bail!("Invalid skill file path");
            }
            let dest = dir.join(path);
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::write(
                &dest,
                STANDARD.decode(
                    file["data"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing skill data"))?,
                )?,
            )?;
            std::fs::set_permissions(
                dest,
                std::fs::Permissions::from_mode(if file["executable"] == true {
                    0o755
                } else {
                    0o644
                }),
            )?;
        }
    }
    Ok(())
}

pub fn env_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !["RALLY_", "CODEX_", "LD_", "DYLD_"]
            .iter()
            .any(|prefix| key.starts_with(prefix))
        && !matches!(
            key,
            "PATH" | "HOME" | "BASH_ENV" | "ENV" | "SHELLOPTS" | "NODE_OPTIONS"
        )
}
