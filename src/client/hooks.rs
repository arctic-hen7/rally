use super::*;
use rally_teams::{digest, id};
use std::{
    fs,
    process::{Command, Stdio},
};

fn settings(adapter: &str) -> Result<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME")?);
    match adapter {
        "codex" => Ok(std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or(home.join(".codex"))
            .join("hooks.json")),
        "claude" => Ok(std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or(home.join(".claude"))
            .join("settings.json")),
        _ => bail!("Choose codex or claude"),
    }
}
pub fn manage(op: &str, adapter: &str) -> Result<()> {
    let path = settings(adapter)?;
    let mut doc = read_json(&path)?;
    let cmd = format!("rally hooks capture {adapter}");
    if doc.get("hooks").is_none() {
        doc["hooks"] = json!({});
    }
    for event in ["UserPromptSubmit", "Stop"] {
        if doc["hooks"].get(event).is_none() {
            doc["hooks"][event] = json!([]);
        }
        let rules = doc["hooks"][event]
            .as_array_mut()
            .context("Invalid hooks configuration")?;
        let match_rule = |r: &Value| {
            r["hooks"]
                .as_array()
                .is_some_and(|hs| hs.iter().any(|h| h["command"] == cmd))
        };
        let found = rules.iter().any(match_rule);
        match op {
            "hooks.install" if !found => {
                rules.push(json!({"hooks":[{"type":"command","command":cmd}]}))
            }
            "hooks.uninstall" => {
                for r in rules.iter_mut() {
                    if let Some(h) = r["hooks"].as_array_mut() {
                        h.retain(|v| v["command"] != cmd);
                    }
                }
                rules.retain(|r| r["hooks"].as_array().is_none_or(|v| !v.is_empty()));
            }
            "hooks.status" => println!("{event}: {}", if found { "installed" } else { "absent" }),
            _ => {}
        }
    }
    if op != "hooks.status" {
        write_json(&path, &doc)?;
        println!("Updated {}", path.display());
        if adapter == "codex" && op == "hooks.install" {
            println!("Review the new hook in Codex /hooks.");
        }
    }
    Ok(())
}
pub fn capture(adapter: &str) -> Result<()> {
    let d: Value = serde_json::from_reader(io::stdin().lock())?;
    let cwd = match d["cwd"].as_str() {
        Some(p) => Path::new(p),
        None => return Ok(()),
    };
    let url = profile_url(&read_json(&config_dir().join("config.json"))?);
    let Some(mid) = workspace::current(cwd, &url)? else {
        return Ok(());
    };
    let event = opt(&d, "hook_event_name");
    let session = field(&d, "session_id")?;
    let (role, text) = match event {
        "UserPromptSubmit" => ("user", opt(&d, "prompt").to_string()),
        "Stop" => ("assistant", last_reply(&d, adapter)?),
        _ => return Ok(()),
    };
    if text.is_empty() {
        return Ok(());
    }
    let dir = config_dir().join("outbox");
    fs::create_dir_all(&dir)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let turn = d["turn_id"].as_str().map(str::to_owned).unwrap_or_else(|| {
        d["transcript_path"]
            .as_str()
            .and_then(|p| fs::metadata(p).ok())
            .map(|m| m.len().to_string())
            .unwrap_or_else(|| timestamp.to_string())
    });
    let key = digest(&format!("{session}|{turn}|{role}|{text}"));
    let title = if role == "user" {
        text.lines()
            .next()
            .unwrap_or(adapter)
            .chars()
            .take(70)
            .collect()
    } else {
        format!("{adapter} session")
    };
    let payload = json!({"mission":mid,"source":adapter,"session":session,"title":title,"role":role,"body":text,"key":key,"server":url,"commits":workspace::commits(cwd,&url,&mid)?});
    write_json(
        &dir.join(format!("{timestamp:032}-{}.json", id("e"))),
        &payload,
    )?;
    Command::new(std::env::current_exe()?)
        .args(["hooks", "flush"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
fn last_reply(d: &Value, adapter: &str) -> Result<String> {
    if let Some(s) = d["last_assistant_message"].as_str() {
        return Ok(s.into());
    }
    let Some(path) = d["transcript_path"].as_str() else {
        return Ok(String::new());
    };
    if adapter != "claude" {
        return Ok(String::new());
    }
    use std::io::{BufRead, BufReader};
    let mut last = String::new();
    for line in BufReader::new(fs::File::open(path)?).lines() {
        let Ok(v) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        if v["type"] == "assistant"
            && let Some(blocks) = v["message"]["content"].as_array()
        {
            let text = blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                last = text;
            }
        }
    }
    Ok(last)
}
pub fn flush(c: &Client) -> Result<usize> {
    let dir = config_dir().join("outbox");
    if !dir.is_dir() {
        return Ok(0);
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".lock"))?;
    lock.lock()?;
    let mut paths = dir
        .read_dir()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|s| s == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut n = 0;
    for p in paths {
        let event = read_json(&p)?;
        if event["server"] != c.url {
            continue;
        }
        c.api("capture", &event)?;
        fs::remove_file(p)?;
        n += 1;
    }
    Ok(n)
}
