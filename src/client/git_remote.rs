// Git calls this transport only inside hosted workers
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{os::unix::process::CommandExt, process::Command};

pub fn serve(name: &str, original: &str) -> Result<()> {
    let rules: rally_teams::GitRoutes = serde_json::from_str(&std::env::var("RALLY_GIT_ROUTES")?)?;
    let url = if let Some(remote) = rally_teams::rewrite_git_url(&rules, original) {
        format!(
            "{}/@routes/{}",
            std::env::var("RALLY_GIT_PROXY")?,
            URL_SAFE_NO_PAD.encode(remote)
        )
    } else if original.starts_with("https://") || original.starts_with("http://") {
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
