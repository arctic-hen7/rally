use super::engine;
use super::*;
use rally_teams::{remote, slug};
use std::path::Path;

pub fn workspace(app: &App, mid: &str) -> PathBuf {
    app.root.join("missions").join(mid)
}

async fn command(app: &App, root: &Path, repo: Option<&str>, args: &[&str]) -> Result<String> {
    let mut cmd = vec![
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "user.name=Rally",
        "-c",
        "user.email=rally@localhost",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "protocol.ext.allow=never",
    ];
    if let Some(name) = repo {
        cmd.extend(["-C", name]);
    }
    cmd.extend(args);
    engine::output(engine::command(
        app,
        root,
        None,
        &[("GIT_TERMINAL_PROMPT".into(), "0".into())],
        None,
        "git",
        &cmd,
    )?)
    .await
}

async fn exists(app: &App, root: &Path, name: &str, revision: &str) -> bool {
    command(app, root, Some(name), &["rev-parse", "--verify", revision])
        .await
        .is_ok()
}
async fn contained(app: &App, root: &Path, name: &str, a: &str, b: &str) -> bool {
    command(
        app,
        root,
        Some(name),
        &["merge-base", "--is-ancestor", a, b],
    )
    .await
    .is_ok()
}

async fn reconcile(app: &App, r: &Value, idle: bool) -> Result<Value> {
    let mid = field(r, "mission")?;
    let root = workspace(app, mid);
    let name = field(r, "name")?;
    let branch = field(r, "branch")?;
    let remote_name = field(r, "remote")?;
    tokio::fs::create_dir_all(&root).await?;
    let run = |args: Vec<String>| {
        let root = root.clone();
        async move {
            let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            command(app, &root, Some(name), &refs).await
        }
    };
    if !root.join(name).join(".git").exists() {
        let base = r["status"]["fork_from"].as_str().unwrap_or(branch);
        command(
            app,
            &root,
            None,
            &["clone", "--branch", base, "--", remote_name, name],
        )
        .await?;
        if base != branch {
            command(app, &root, Some(name), &["switch", "-c", branch]).await?;
            command(app, &root, Some(name), &["push", "-u", "origin", branch]).await?;
        }
    }
    command(app, &root, Some(name), &["fetch", "--prune", "origin"]).await?;
    let mut active = command(app, &root, Some(name), &["branch", "--show-current"]).await?;
    let dirty = !command(app, &root, Some(name), &["status", "--porcelain"])
        .await?
        .is_empty();
    let unmerged = !command(app, &root, Some(name), &["ls-files", "--unmerged"])
        .await?
        .is_empty();
    let original = format!("refs/remotes/origin/{branch}");
    let mut recovery = r["recovery"].as_str();
    if let Some(wip) = recovery {
        let local_wip = format!("refs/heads/{wip}");
        let remote_wip = format!("refs/remotes/origin/{wip}");
        let published = exists(app, &root, name, &remote_wip).await;
        if idle
            && !dirty
            && active == wip
            && contained(app, &root, name, &local_wip, &original).await
            && (!published || contained(app, &root, name, &remote_wip, &original).await)
            && contained(app, &root, name, branch, &original).await
        {
            command(app, &root, Some(name), &["switch", branch]).await?;
            command(app, &root, Some(name), &["merge", "--ff-only", &original]).await?;
            if published {
                let expected = command(app, &root, Some(name), &["rev-parse", &remote_wip]).await?;
                run(vec![
                    "push".into(),
                    format!("--force-with-lease=refs/heads/{wip}:{expected}"),
                    "origin".into(),
                    format!(":refs/heads/{wip}"),
                ])
                .await?;
            }
            command(app, &root, Some(name), &["branch", "-d", wip]).await?;
            execute(
                &app.db,
                "UPDATE repos SET recovery=NULL WHERE mission=$1 AND name=$2",
                &[mid, name],
            )
            .await?;
            active = branch.to_string();
            recovery = None;
        }
    }
    let expected = recovery.unwrap_or(branch);
    let tracking = format!("refs/remotes/origin/{expected}");
    let mut status = "current";
    let (mut ahead, mut behind) = (0_u64, 0_u64);
    if active != expected {
        status = "different branch";
    } else if !exists(app, &root, name, &tracking).await {
        status = "unpublished";
    } else {
        let counts = command(
            app,
            &root,
            Some(name),
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("HEAD...{tracking}"),
            ],
        )
        .await?;
        let mut n = counts.split_whitespace().map(str::parse::<u64>);
        ahead = n.next().unwrap()?;
        behind = n.next().unwrap()?;
        if unmerged {
            status = "unresolved merge";
        } else if ahead > 0 && behind > 0 {
            status = "diverged";
        } else if behind > 0 && dirty {
            status = "local edits";
        } else if behind > 0 && idle {
            command(app, &root, Some(name), &["merge", "--ff-only", &tracking]).await?;
            behind = 0;
        } else if behind > 0 {
            status = "update pending";
        } else if ahead > 0 {
            status = "unpushed";
        } else if dirty {
            status = "modified";
        }
    }
    let head = command(app, &root, Some(name), &["rev-parse", "HEAD"]).await?;
    Ok(
        json!({"state":status,"head":head,"branch":active,"ahead":ahead,"behind":behind,"dirty":dirty}),
    )
}

pub async fn sync(app: &App, mid: &str) -> Result<Value> {
    let _guard = app.git_lock.lock().await;
    let idle:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread WHERE t.mission=$1 AND r.status IN ('working','stopping'))").bind(mid).fetch_one(&app.db).await?;
    let repos = rows(
        &app.db,
        "SELECT to_jsonb(r) FROM repos r WHERE mission=$1 ORDER BY name",
        &[mid],
    )
    .await?;
    for r in repos.as_array().unwrap() {
        let name = field(r, "name")?;
        let status = match reconcile(app, r, idle).await {
            Ok(s) => s,
            Err(e) => {
                let mut s = r["status"].clone();
                s["state"] = json!("error");
                s["error"] = json!(e.to_string());
                s
            }
        };
        sqlx::query("UPDATE repos SET status=$3 WHERE mission=$1 AND name=$2")
            .bind(mid)
            .bind(name)
            .bind(status)
            .execute(&app.db)
            .await?;
    }
    rows(
        &app.db,
        "SELECT to_jsonb(r) FROM repos r WHERE mission=$1 ORDER BY name",
        &[mid],
    )
    .await
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    let mid = mission(app, a, field(d, "mission")?, op != "repo.list").await?;
    let root = workspace(app, &mid);
    if op == "repo.list" {
        return rows(
            &app.db,
            "SELECT to_jsonb(r) FROM repos r WHERE mission=$1 ORDER BY name",
            &[&mid],
        )
        .await;
    }
    personal(a)?;
    match op {
        "repo.add" => {
            let name = slug(field(d, "name")?)?;
            let branch = field(d, "branch")?;
            if branch.starts_with('-')
                || !tokio::process::Command::new("git")
                    .args(["check-ref-format", "--branch", branch])
                    .output()
                    .await?
                    .status
                    .success()
            {
                bail!("Invalid branch");
            }
            execute(
                &app.db,
                "INSERT INTO repos(mission,name,remote,branch) VALUES($1,$2,$3,$4)",
                &[&mid, name, remote(field(d, "remote")?)?, branch],
            )
            .await?;
        }
        "repo.remove" => {
            execute(
                &app.db,
                "DELETE FROM repos WHERE mission=$1 AND name=$2",
                &[&mid, field(d, "name")?],
            )
            .await?;
        }
        "repo.wip" | "repo.return" => {
            let _guard = app.git_lock.lock().await;
            let name = slug(field(d, "name")?)?;
            let r = one(
                &app.db,
                "SELECT to_jsonb(r) FROM repos r WHERE mission=$1 AND name=$2",
                &[&mid, name],
            )
            .await?;
            if op == "repo.return" {
                if r["recovery"].is_null() {
                    bail!("No recovery branch");
                }
                if !command(app, &root, Some(name), &["status", "--porcelain"])
                    .await?
                    .is_empty()
                {
                    bail!("Commit local edits before returning");
                }
                command(app, &root, Some(name), &["switch", field(&r, "branch")?]).await?;
                execute(
                    &app.db,
                    "UPDATE repos SET recovery=NULL WHERE mission=$1 AND name=$2",
                    &[&mid, name],
                )
                .await?;
            } else {
                if !command(app, &root, Some(name), &["ls-files", "--unmerged"])
                    .await?
                    .is_empty()
                {
                    bail!("Resolve the merge before committing");
                }
                let mut summary = opt(d, "summary").to_string();
                if summary.is_empty() {
                    let diff = command(app, &root, Some(name), &["diff", "HEAD", "--stat"]).await?;
                    let paths = command(app, &root, Some(name), &["status", "--short"]).await?;
                    let history=rows(&app.db,"SELECT to_jsonb(body) FROM messages WHERE thread=COALESCE(NULLIF($2,''),(SELECT id FROM threads WHERE mission=$1 AND kind='main')) AND thread IN(SELECT id FROM threads WHERE mission=$1) ORDER BY id DESC LIMIT 8",&[&mid,opt(d,"thread")]).await?;
                    let prompt = format!(
                        "Write only one concise Git WIP commit subject. Files: {paths}\nDiff: {diff}\nRecent conversation: {history}"
                    );
                    summary = super::runtime::summary(app, &root, &prompt).await?;
                }
                if d["fork"] == true && r["recovery"].is_null() {
                    let wip = format!("rally/wip/{mid}-{name}-{}", &id("b")[2..10]);
                    command(app, &root, Some(name), &["switch", "-c", &wip]).await?;
                    execute(
                        &app.db,
                        "UPDATE repos SET recovery=$3 WHERE mission=$1 AND name=$2",
                        &[&mid, name, &wip],
                    )
                    .await?;
                }
                command(app, &root, Some(name), &["add", "--all"]).await?;
                command(
                    app,
                    &root,
                    Some(name),
                    &[
                        "commit",
                        "--no-gpg-sign",
                        "-m",
                        &format!(
                            "wip: {}",
                            summary.lines().next().unwrap_or("work in progress")
                        ),
                    ],
                )
                .await?;
                if d["fork"] == true {
                    command(app, &root, Some(name), &["push", "-u", "origin", "HEAD"]).await?;
                }
            }
        }
        "repo.push" => {
            let name = slug(field(d, "name")?)?;
            one(
                &app.db,
                "SELECT to_jsonb(name) FROM repos WHERE mission=$1 AND name=$2",
                &[&mid, name],
            )
            .await?;
            command(app, &root, Some(name), &["push", "-u", "origin", "HEAD"]).await?;
        }
        "repo.sync" => {}
        _ => bail!("Unknown repository operation"),
    }
    sync(app, &mid).await
}
