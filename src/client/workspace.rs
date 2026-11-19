use super::*;
use rally_teams::{digest, slug};
use std::{fs, os::unix::fs::symlink, process::Command};

fn locked_registry<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&mut Value) -> Result<T>,
{
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("missions.lock"))?;
    lock.lock()?;
    let file = dir.join("missions.json");
    let mut records = read_json(&file)?;
    let old = records.clone();
    records
        .as_object_mut()
        .context("Invalid missions.json")?
        .retain(|path, r| {
            read_json(&Path::new(path).join(".rally/mission.json")).is_ok_and(|v| v == *r)
        });
    let result = f(&mut records)?;
    if old != records {
        write_json(&file, &records)?;
    }
    Ok(result)
}
pub fn registered(url: &str) -> Result<Value> {
    locked_registry(|records| {
        Ok(json!(
            records
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, r)| r["url"] == url)
                .map(|(p, r)| json!({"id":r["mission"],"path":p}))
                .collect::<Vec<_>>()
        ))
    })
}
pub fn current(cwd: &Path, url: &str) -> Result<Option<String>> {
    let file = config_dir().join("missions.json");
    if !file.exists() {
        return Ok(None);
    }
    let cwd = cwd.canonicalize()?;
    let regs = read_json(&file)?;
    for p in cwd.ancestors() {
        let Some(r) = regs.get(p.to_string_lossy().as_ref()) else {
            continue;
        };
        if r["url"] == url && read_json(&p.join(".rally/mission.json"))? == *r {
            return Ok(r["mission"].as_str().map(str::to_owned));
        }
    }
    Ok(None)
}
pub fn path(c: &Client, mid: &str) -> Result<PathBuf> {
    let mid = mid.trim_start_matches('#');
    for row in registered(&c.url)?.as_array().unwrap() {
        if row["id"] == mid {
            return Ok(PathBuf::from(field(row, "path")?));
        }
    }
    bail!("Mission has not been opened locally")
}
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().into())
}
pub fn open(c: &Client, mid: &str) -> Result<PathBuf> {
    let m = c.api("mission.show", &json!({"mission":mid}))?;
    let mid = slug(field(&m, "id")?)?;
    let cfg = read_json(&config_dir().join("config.json"))?;
    let root = match path(c, mid) {
        Ok(p) => p,
        Err(_) => {
            if let Some(dir) = cfg["directory"].as_str().filter(|s| !s.is_empty()) {
                let base = if let Some(rel) = dir.strip_prefix("~/") {
                    PathBuf::from(std::env::var("HOME")?).join(rel)
                } else {
                    dir.into()
                };
                base.join(mid)
            } else {
                std::env::current_dir()?
            }
        }
    };
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let marker = root.join(".rally/mission.json");
    let record = json!({"mission":mid,"url":c.url});
    if marker.exists() {
        if read_json(&marker)? != record {
            bail!("Directory belongs to another Rally mission");
        }
    } else if root.read_dir()?.next().is_some() {
        bail!(
            "Mission roots must be empty on first open. Configure a default directory with rally config set --key directory --value ~/rally"
        );
    }
    write_json(&marker, &record)?;
    locked_registry(|records| {
        records[root.to_string_lossy().as_ref()] = record;
        Ok(())
    })?;
    let files = c.api("skill.bundle", &json!({"mission":mid}))?;
    let store = root.join(".rally/skills").join(digest(&files.to_string()));
    if !store.exists() {
        let temp = store.with_extension(rally_teams::id("tmp"));
        rally_teams::materialize_skills(&temp, &files)?;
        if let Err(e) = fs::rename(temp, &store)
            && !store.exists()
        {
            return Err(e.into());
        }
    }
    let attach = |dir: &Path| -> Result<()> {
        for harness in [".agents", ".claude"] {
            let dest = dir.join(harness).join("skills");
            fs::create_dir_all(&dest)?;
            if !dest.canonicalize()?.starts_with(&root) {
                bail!("Skill path points outside the mission");
            }
            for entry in dest.read_dir()? {
                let e = entry?;
                if let Ok(old) = fs::read_link(e.path())
                    && old.starts_with(root.join(".rally/skills"))
                {
                    fs::remove_file(e.path())?;
                }
            }
            for name in files.as_object().unwrap().keys() {
                let link = dest.join(name);
                if fs::symlink_metadata(&link).is_err() {
                    symlink(store.join(name), link)?;
                }
            }
        }
        Ok(())
    };
    attach(&root)?;
    for r in m["repos"].as_array().unwrap() {
        let name = slug(field(r, "name")?)?;
        let branch = r["recovery"].as_str().unwrap_or(field(r, "branch")?);
        let repo = root.join(name);
        if !repo.exists() {
            git(
                &root,
                &["clone", "--branch", branch, "--", field(r, "remote")?, name],
            )?;
        } else {
            if git(&repo, &["config", "--get", "remote.origin.url"])? != field(r, "remote")? {
                bail!("{name} has a different remote");
            }
            git(&repo, &["fetch", "--prune", "origin"])?;
            let local = git(&repo, &["branch", "--show-current"])?;
            if git(&repo, &["status", "--porcelain"])?.is_empty() {
                if local != branch {
                    if git(
                        &repo,
                        &[
                            "merge-base",
                            "--is-ancestor",
                            "HEAD",
                            &format!("origin/{branch}"),
                        ],
                    )
                    .is_ok()
                    {
                        git(&repo, &["switch", branch])?;
                    } else {
                        eprintln!(
                            "{name}: local branch {local} has independent work; checkout preserved"
                        );
                    }
                }
                if git(&repo, &["branch", "--show-current"])? == branch
                    && let Err(e) = git(&repo, &["merge", "--ff-only", &format!("origin/{branch}")])
                {
                    eprintln!("{name}: {e}");
                }
            } else {
                eprintln!("{name}: local changes preserved");
            }
        }
        attach(&repo)?;
        let exclude = git(&repo, &["rev-parse", "--git-path", "info/exclude"])?;
        let p = repo.join(exclude);
        let mut text = fs::read_to_string(&p).unwrap_or_default();
        for name in files.as_object().unwrap().keys() {
            for harness in [".agents", ".claude"] {
                let line = format!("/{harness}/skills/{name}");
                if !text.lines().any(|l| l == line) {
                    text.push('\n');
                    text += &line;
                    text.push('\n');
                }
            }
        }
        fs::write(p, text)?;
    }
    write_json(
        &root.join(".rally/repos.json"),
        &json!(
            m["repos"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| &r["name"])
                .collect::<Vec<_>>()
        ),
    )?;
    super::secrets::first_open(c, mid, &root)?;
    Ok(root)
}
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
pub fn shell(shell: &str) -> Result<&'static str> {
    match shell {
        "bash" | "zsh" | "" => Ok(r#"rally() {
    if [ "$1" = open ]; then
        shift
        local rally_path
        rally_path="$(command rally open "$@" --print-path)" && cd -- "$rally_path"
    else
        command rally "$@"
    fi
}"#),
        "fish" => Ok(r#"function rally
    if test "$argv[1]" = open
        set -l rally_path (command rally open $argv[2..-1] --print-path)
        and cd -- "$rally_path"
    else
        command rally $argv
    end
end"#),
        _ => bail!("Supported shells: bash, zsh, fish"),
    }
}

pub fn commits(cwd: &Path, url: &str, mid: &str) -> Result<Value> {
    let regs = read_json(&config_dir().join("missions.json"))?;
    let mut result = json!({});
    for root in cwd.canonicalize()?.ancestors() {
        let Some(r) = regs.get(root.to_string_lossy().as_ref()) else {
            continue;
        };
        if r["mission"] != mid || r["url"] != url {
            continue;
        }
        let names = read_json(&root.join(".rally/repos.json"))?;
        for name in names
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Ok(sha) = git(&root.join(slug(name)?), &["rev-parse", "HEAD"]) {
                result[name] = json!(sha);
            }
        }
        break;
    }
    Ok(result)
}
