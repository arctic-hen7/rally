use super::*;
use std::path::Path;
use tokio::process::Command;

pub fn local() -> bool {
    var("RALLY_ENGINE", "podman") == "local"
}

pub fn engine() -> Command {
    let mut cmd = Command::new(var("RALLY_PODMAN", "podman"));
    if let Ok(socket) = std::env::var("RALLY_PODMAN_URL") {
        cmd.args(["--remote", "--url", &socket]);
    }
    cmd
}

fn installation_label() -> Result<String> {
    let host_data = std::env::var("RALLY_HOST_DATA")?;
    Ok(format!(
        "io.framesurge.rally.installation={}",
        rally_teams::digest(&host_data)
    ))
}

pub async fn build(app: &App) -> Result<()> {
    if local() {
        return Ok(());
    }
    // The database lock ensures previous workers from this installation are orphaned
    let mut owned = engine();
    owned.args([
        "ps",
        "--all",
        "--quiet",
        "--filter",
        &format!("label={}", installation_label()?),
    ]);
    for id in output(owned).await?.lines() {
        let mut remove = engine();
        remove.args(["rm", "--force", "--ignore", id]);
        output(remove).await?;
    }
    let tools = app.root.join("tools");
    tokio::fs::create_dir_all(&tools).await?;
    let cli = std::env::current_exe()?.with_file_name("rally");
    tokio::fs::copy(cli, tools.join("rally.new")).await?;
    tokio::fs::rename(tools.join("rally.new"), tools.join("rally")).await?;
    build_image().await
}

pub async fn build_image() -> Result<()> {
    let context = var("RALLY_BUILD_DIR", "runtime");
    let status = engine()
        .args([
            "build",
            "--tag",
            &var("RALLY_RUNTIME_IMAGE", "localhost/rally-runtime:local"),
            "--file",
            &format!("{context}/Containerfile"),
            &context,
        ])
        .status()
        .await?;
    if !status.success() {
        bail!("Rally runtime image build failed");
    }
    Ok(())
}

pub fn command(
    app: &App,
    workspace: &Path,
    home: Option<&Path>,
    env: &[(String, String)],
    name: Option<&str>,
    program: &str,
    args: &[&str],
) -> Result<Command> {
    if local() {
        let mut c = Command::new(program);
        c.args(args)
            .current_dir(workspace)
            .envs(env.iter().cloned());
        if let Some(home) = home {
            c.env("CODEX_HOME", home);
        }
        return Ok(c);
    }
    let host_root = PathBuf::from(std::env::var("RALLY_HOST_DATA")?);
    let host_path = |p: &Path| -> Result<String> {
        let rel = p.strip_prefix(&app.root)?;
        Ok(host_root.join(rel).to_string_lossy().into_owned())
    };
    let mut c = engine();
    c.args([
        "run",
        "--rm",
        "--interactive",
        "--cap-drop=all",
        "--security-opt=no-new-privileges",
        "--label",
        "io.framesurge.rally.managed=true",
        "--label",
        &installation_label()?,
        "--network",
        &var("RALLY_NETWORK", "rally_default"),
        "--workdir",
        "/workspace",
        "--volume",
        &format!("{}:/workspace:rw", host_path(workspace)?),
    ]);
    c.args([
        "--volume",
        &format!(
            "{}:/usr/local/bin/rally:ro",
            host_root.join("tools/rally").display()
        ),
    ]);
    if let Some(name) = name {
        c.args(["--name", name]);
    }
    if let Some(home) = home {
        c.args([
            "--volume",
            &format!("{}:/session:rw", host_path(home)?),
            "--env",
            "CODEX_HOME=/session",
        ]);
    }
    if let Some((_, rid)) = env.iter().find(|(k, _)| k == "RALLY_RUN") {
        rally_teams::slug(rid)?;
        c.args([
            "--volume",
            &format!(
                "{}:/rally/context:ro",
                host_root.join("contexts").join(rid).display()
            ),
            "--volume",
            &format!(
                "{}:/root/.agents/skills:ro",
                host_root
                    .join("contexts")
                    .join(rid)
                    .join("skills")
                    .display()
            ),
        ]);
    }
    let mounts: Vec<String> = serde_json::from_str(&var("RALLY_RUNTIME_MOUNTS", "[]"))?;
    for mount in mounts {
        c.args(["--volume", &mount]);
    }
    for (key, value) in env {
        c.env(key, value).args(["--env", key]);
    }
    c.arg(var("RALLY_RUNTIME_IMAGE", "localhost/rally-runtime:local"))
        .arg(program)
        .args(args);
    Ok(c)
}

pub async fn output(mut cmd: Command) -> Result<String> {
    cmd.kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output()).await??;
    if !out.status.success() {
        bail!(
            "{}",
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(1600)
                .collect::<String>()
                .trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

pub async fn stop(name: &str) {
    if !local() {
        let _ = engine().args(["stop", "--time", "10", name]).output().await;
    }
}
