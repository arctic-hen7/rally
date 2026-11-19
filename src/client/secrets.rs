use super::*;
use std::{collections::BTreeMap, fs, io::Write, path::Path, process::Command};
use tempfile::NamedTempFile;

fn write_env(root: &Path, entries: &Value) -> Result<()> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut baseline = json!({});
    for (key, v) in entries.as_object().context("Invalid secrets response")? {
        let value = v["value"]
            .as_str()
            .with_context(|| format!("Resolve conflicting skill secret {key} before pulling"))?;
        let source = field(v, "source")?;
        let label = v["sources"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(|s| s["name"].as_str())
            .unwrap_or(source);
        let label: String = label.chars().filter(|c| !c.is_control()).collect();
        let quoted = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        let block = groups
            .entry(format!(
                "{}: {label}",
                if source.starts_with("sk_") {
                    "Skill"
                } else {
                    "Mission"
                }
            ))
            .or_default();
        if let Some(sources) = v["sources"].as_array().filter(|s| s.len() > 1) {
            let ids = sources[..sources.len() - 1]
                .iter()
                .filter_map(|s| s["id"].as_str())
                .collect::<Vec<_>>();
            block.push(format!("# Overrides {}", ids.join(", ")));
        }
        block.push(format!("{key}=\"{quoted}\""));
        baseline[key] = json!({"checksum":v["checksum"],"source":source});
    }
    let mut file = NamedTempFile::new_in(root)?;
    for (source, lines) in groups {
        writeln!(file, "# {source}\n{}\n", lines.join("\n"))?;
    }
    file.as_file().sync_all()?;
    file.persist(root.join(".rally.env"))?;
    write_json(&root.join(".rally/secrets.json"), &baseline)?;
    Ok(())
}
fn read_env(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (i, line) in fs::read_to_string(path)?.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, raw) = line
            .split_once('=')
            .with_context(|| format!("Expected KEY=value on line {}", i + 1))?;
        let key = key.trim();
        if !rally_teams::env_key(key) {
            bail!("Invalid environment key on line {}", i + 1);
        }
        let raw = raw.trim();
        let value = if let Some(quoted) = raw.strip_prefix('"') {
            let mut chars = quoted.chars();
            let mut value = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        let tail = chars.as_str().trim();
                        if !tail.is_empty() && !tail.starts_with('#') {
                            bail!("Unexpected text after {key}");
                        }
                        closed = true;
                        break;
                    }
                    '\\' => value.push(match chars.next() {
                        Some('n') => '\n',
                        Some('r') => '\r',
                        Some('t') => '\t',
                        Some(c @ ('"' | '\\' | '$' | '`')) => c,
                        _ => bail!("Invalid escape in {key}"),
                    }),
                    _ => value.push(c),
                }
            }
            if !closed {
                bail!("Unclosed value for {key}");
            }
            value
        } else if let Some(s) = raw.strip_prefix('\'') {
            s.strip_suffix('\'')
                .with_context(|| format!("Unclosed value for {key}"))?
                .to_owned()
        } else {
            raw.split_once(" #")
                .map(|(v, _)| v)
                .unwrap_or(raw)
                .to_owned()
        };
        if out.insert(key.to_owned(), value).is_some() {
            bail!("Duplicate key {key}");
        }
    }
    Ok(out)
}

pub fn first_open(c: &Client, mid: &str, root: &Path) -> Result<()> {
    if root.join(".rally.env").symlink_metadata().is_ok() {
        return Ok(());
    }
    let entries = c.api("secrets.list", &json!({"mission":mid}))?;
    let within = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output()?
        .status
        .success();
    if within
        && !Command::new("git")
            .args(["check-ignore", "-q", "--", ".rally.env"])
            .current_dir(root)
            .status()?
            .success()
    {
        eprintln!("Warning: .rally.env is not ignored by Git in this checkout.");
    }
    let ignore = root.join(".gitignore");
    if ignore
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        bail!("Refusing to edit a linked mission .gitignore");
    }
    let existing = match fs::read_to_string(&ignore) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    if !existing.lines().any(|s| s == "/.rally.env") {
        writeln!(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(ignore)?,
            "\n/.rally.env\n/.rally/secrets.json"
        )?;
    }
    write_env(root, &entries)
}
pub fn sync(c: &Client, op: &str, d: &Value) -> Result<Value> {
    let mid = field(d, "mission")?;
    let root = workspace::path(c, mid)?;
    let current = c.api("secrets.list", &json!({"mission":mid}))?;
    if op == "secrets.pull" {
        write_env(&root, &current)?;
        return Ok(json!({"pulled":true}));
    }
    let base = read_json(&root.join(".rally/secrets.json"))?;
    let entries = read_env(&root.join(".rally.env"))?;
    let mut changes = json!({});
    let mut conflicts = vec![];
    let keys: std::collections::BTreeSet<_> = entries
        .keys()
        .chain(base.as_object().context("Invalid checksum file")?.keys())
        .collect();
    for key in keys {
        let old = &base[key];
        let upstream = &current[key];
        let value = entries.get(key);
        let hash = json!(value.map(|v| rally_teams::digest(v)));
        if hash == old["checksum"] || hash == upstream["checksum"] {
            continue;
        }
        if (old["checksum"] != upstream["checksum"] || old["source"] != upstream["source"])
            && !d["overwrite"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|k| k == key))
        {
            conflicts.push(key.as_str());
            continue;
        }
        changes[key] =
            json!({"value":value,"base":upstream["checksum"],"source":upstream["source"]});
    }
    if !conflicts.is_empty() {
        bail!(
            "Changed upstream: {}. Reconcile or use --overwrite KEY for each intended replacement.",
            conflicts.join(", ")
        );
    }
    let result = c.api("secrets.apply", &json!({"mission":mid,"changes":changes}))?;
    write_env(&root, &result)?;
    Ok(json!({"pushed":changes.as_object().unwrap().keys().collect::<Vec<_>>()}))
}
