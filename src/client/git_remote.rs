// Git calls this transport only inside hosted workers
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{collections::BTreeMap, os::unix::process::CommandExt, process::Command};

pub fn serve(name: &str, original: &str) -> Result<()> {
    let rules: BTreeMap<String, (String, bool)> =
        serde_json::from_str(&std::env::var("RALLY_GIT_ROUTES")?)?;
    let original = original.trim_end_matches('/');
    let stem = original.strip_suffix(".git").unwrap_or(original);
    let matched = rules
        .iter()
        .filter(|(source, (_, prefix))| {
            if *prefix {
                original.starts_with(source.as_str())
            } else {
                stem == source.as_str()
            }
        })
        .max_by_key(|(source, _)| source.len());
    let url = if let Some((source, (upstream, prefix))) = matched {
        let remote = if *prefix {
            format!(
                "{}/{}",
                upstream.trim_end_matches('/'),
                &original[source.len()..]
            )
        } else {
            upstream.clone()
        };
        format!(
            "{}/@routes/{}",
            std::env::var("RALLY_GIT_PROXY")?,
            URL_SAFE_NO_PAD.encode(remote)
        )
    } else if original.starts_with("https://") {
        // Git insteadOf uses raw prefixes, so a neighbouring repository can reach the helper
        original.to_owned()
    } else {
        bail!("This SSH repository is outside the configured Rally Git routes");
    };
    let transport = url
        .split_once("://")
        .context("Invalid Git transport URL")?
        .0;
    ensure!(
        matches!(transport, "http" | "https"),
        "Invalid Git transport"
    );
    Err(Command::new("git")
        .arg(format!("remote-{transport}"))
        .arg(name)
        .arg(url)
        .exec()
        .into())
}
