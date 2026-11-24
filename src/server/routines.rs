use super::*;
use super::{engine, git, notify, search, skills};
use rally_teams::{digest, remote, secret, slug};
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

fn directory(value: Option<&str>) -> Result<&str> {
    let p = value.filter(|s| !s.is_empty()).unwrap_or(".");
    if p != "." {
        skills::relative(p)?;
    }
    Ok(p)
}
async fn validate_repos(db: &mut sqlx::PgConnection, owner: &str, repos: &Value) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    for r in repos
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Repos must be an array"))?
    {
        if !names.insert(slug(field(r, "name")?)?) {
            bail!("Checkout names must be unique");
        }
        if r["needs_connection"] == true {
            bail!("Reconnect the transferred repository or explicitly select anonymous access");
        }
        connections::validate(
            &mut *db,
            owner,
            field(r, "remote")?,
            r["connection"].as_str().filter(|s| !s.is_empty()),
        )
        .await?;
        remote(field(r, "remote")?)?;
        if field(r, "branch")?.starts_with('-') {
            bail!("Invalid branch");
        }
    }
    Ok(())
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    if a.routine.is_some() && matches!(op, "routine.create" | "routine.update") {
        bail!("Manage routine workspaces with a user or service account");
    }
    if op == "routine.preview" {
        return Ok(json!(triggers::upcoming(
            field(d, "cron")?,
            field(d, "timezone")?
        )?));
    }
    if op == "routine.list" {
        return rows(
            &app.db,
            "SELECT to_jsonb(r) FROM routines r WHERE owner_id=$1 ORDER BY name",
            &[&a.uid],
        )
        .await;
    }
    let mut tx = app.db.begin().await?;
    let rid = if op == "routine.create" {
        id("rt")
    } else {
        field(d, "routine")?.to_owned()
    };
    if op == "routine.create" {
        let repos = d.get("repos").cloned().unwrap_or(json!([]));
        validate_repos(&mut tx, &a.uid, &repos).await?;
        sqlx::query("INSERT INTO routines(id,name,owner_id,command,repos,directory) VALUES($1,$2,$3,$4,$5,$6)").bind(&rid).bind(field(d,"name")?).bind(&a.uid).bind(field(d,"command")?).bind(repos).bind(directory(d["directory"].as_str())?).execute(&mut *tx).await?;
    }
    let mut r: Value =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM routines r WHERE id=$1 FOR UPDATE")
            .bind(&rid)
            .fetch_one(&mut *tx)
            .await?;
    if op.starts_with("routine.transfer") {
        admin(a)?;
        // Serialize changes to subscriptions before counting an endpoint's subscribers
        sqlx::query(
            "SELECT w.id FROM webhooks w JOIN routine_triggers t ON t.webhook=w.id
             WHERE t.routine=$1 ORDER BY w.id FOR UPDATE OF w",
        )
        .bind(&rid)
        .execute(&mut *tx)
        .await?;
        let hooks: Vec<Value> = sqlx::query_scalar(
            "SELECT jsonb_build_object('id',w.id,'name',w.name,'exclusive',
                (SELECT count(*)=1 FROM routine_triggers WHERE webhook=w.id))
             FROM webhooks w JOIN routine_triggers t ON t.webhook=w.id
             WHERE t.routine=$1 ORDER BY w.id",
        )
        .bind(&rid)
        .fetch_all(&mut *tx)
        .await?;
        if op == "routine.transfer-preview" {
            return Ok(json!({"hooks":hooks}));
        }
        let recipient = field(d, "person")?;
        if recipient == field(&r, "owner_id")? {
            bail!("Choose a different owner");
        }
        if !hooks.is_empty() && d["confirm"] != true {
            bail!("Confirm webhook detachment or handover before transfer");
        }
        for w in &hooks {
            let hook = field(w, "id")?;
            if w["exclusive"] == true && d["handover"] == true {
                sqlx::query("UPDATE webhooks SET owner_id=$2 WHERE id=$1")
                    .bind(hook)
                    .bind(recipient)
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("DELETE FROM routine_triggers WHERE routine=$1 AND webhook=$2")
                    .bind(&rid)
                    .bind(hook)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let mut repos = r["repos"].clone();
        let mut pause = false;
        for repo in repos.as_array_mut().unwrap() {
            if repo["connection"].as_str().is_some_and(|s| !s.is_empty())
                || repo["needs_connection"] == true
            {
                repo.as_object_mut().unwrap().remove("connection");
                repo["needs_connection"] = json!(true);
                pause = true;
            }
        }
        sqlx::query("UPDATE routines SET repos=$2,enabled=enabled AND NOT $3 WHERE id=$1")
            .bind(&rid)
            .bind(repos)
            .bind(pause)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE jobs SET status='aborted',token_digest=NULL WHERE routine=$1 AND status='queued'").bind(&rid).execute(&mut *tx).await?;
        sqlx::query("UPDATE routines SET owner_id=$2 WHERE id=$1")
            .bind(&rid)
            .bind(recipient)
            .execute(&mut *tx)
            .await?;
    } else {
        if r["owner_id"] != a.uid {
            bail!("Access denied");
        }
        match op {
            "routine.show" => {
                r["triggers"] = json!(
                    sqlx::query_scalar::<_, Value>(
                        "SELECT to_jsonb(t) FROM routine_triggers t WHERE routine=$1 ORDER BY id"
                    )
                    .bind(&rid)
                    .fetch_all(&mut *tx)
                    .await?
                );
                r["jobs"]=json!(sqlx::query_scalar::<_,Value>("SELECT to_jsonb(j)-'token_digest'-'payload'-'definition' FROM jobs j WHERE routine=$1 AND owner_id=$2 ORDER BY created DESC LIMIT 20").bind(&rid).bind(&a.uid).fetch_all(&mut *tx).await?);
                return Ok(r);
            }
            "routine.create" | "routine.update" => {
                if d.get("cron").is_some() {
                    bail!("Supply a triggers array for webhook and cron triggers");
                }
                if let Some(list) = d.get("triggers") {
                    triggers::replace(&mut tx, &rid, &a.uid, list).await?;
                }
                if op == "routine.update" {
                    let repos = d.get("repos").unwrap_or(&r["repos"]);
                    validate_repos(&mut tx, &a.uid, repos).await?;
                    sqlx::query("UPDATE routines SET name=coalesce($2,name),command=coalesce($3,command),repos=$4,directory=$5 WHERE id=$1").bind(&rid).bind(d["name"].as_str()).bind(d["command"].as_str()).bind(repos).bind(directory(d["directory"].as_str().or(r["directory"].as_str()))?).execute(&mut *tx).await?;
                }
            }
            "routine.trigger" => {
                let job = triggers::enqueue(&mut tx, &r, &d["payload"]).await?;
                tx.commit().await?;
                return Ok(json!({"id":job}));
            }
            "routine.enable" | "routine.disable" => {
                if op == "routine.enable" {
                    validate_repos(&mut tx, &a.uid, &r["repos"]).await?;
                }
                sqlx::query("UPDATE routines SET enabled=$2 WHERE id=$1")
                    .bind(&rid)
                    .bind(op == "routine.enable")
                    .execute(&mut *tx)
                    .await?;
            }
            _ => bail!("Unknown routine operation"),
        }
    }
    tx.commit().await?;
    Ok(json!({"id":rid}))
}

pub async fn maintenance(app: App) -> Result<()> {
    execute(
        &app.db,
        "UPDATE jobs SET status='aborted',token_digest=NULL WHERE status='working'",
        &[],
    )
    .await?;
    let mut tasks = tokio::task::JoinSet::new();
    let mut stop = app.shutdown.clone();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let upkeep = tokio::spawn(upkeep(app.clone()));
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            _=tick.tick()=>{
                triggers::schedule(&app).await?;
                while tasks.len()<4{
                    let token=secret();
                    let row=sqlx::query("WITH p AS (SELECT id FROM jobs WHERE status='queued' ORDER BY created LIMIT 1 FOR UPDATE SKIP LOCKED) UPDATE jobs j SET status='working',token_digest=$1 FROM p WHERE j.id=p.id RETURNING j.id").bind(digest(&token)).fetch_optional(&app.db).await?;
                    let Some(row)=row else{break};let jid:String=row.get("id");let app=app.clone();
                    tasks.spawn(async move{
                        let res=run(&app,&jid,&token).await;
                        let (status,out)=match res{Ok((s,o))=>(s,o),Err(e)=>("failed",e.to_string())};
                        let _=execute(&app.db,"UPDATE jobs SET status=$2,output=$3,token_digest=NULL WHERE id=$1",&[&jid,status,&out]).await;
                        let path=app.root.join("jobs").join(jid);
                        if let Err(e)=tokio::fs::remove_dir_all(path).await{eprintln!("Routine workspace cleanup: {e}");}
                    });
                }
            }
            Some(_)=tasks.join_next(),if !tasks.is_empty()=>{}
        }
    }
    while tasks.join_next().await.is_some() {}
    upkeep.abort();
    Ok(())
}

async fn upkeep(app: App) {
    let mut cycle = 0;
    loop {
        if let Err(e) = search::index(&app).await {
            eprintln!("Search indexing: {e}");
        }
        if cycle % 5 == 0
            && let Err(e) = notify::deliver(&app).await
        {
            eprintln!("Push: {e}");
        }
        if cycle % 30 == 0 {
            if let Err(e) = skills::refresh(&app).await {
                eprintln!("Skills: {e}");
            }
            if let Ok(missions) = sqlx::query_scalar::<_, String>(
                "SELECT DISTINCT mission FROM repos WHERE NOT read_only",
            )
            .fetch_all(&app.db)
            .await
            {
                for m in missions {
                    if let Err(e) = git::sync(&app, &m).await {
                        eprintln!("Git sync: {e}");
                    }
                }
            }
        }
        cycle += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn run(app: &App, jid: &str, token: &str) -> Result<(&'static str, String)> {
    let r = one(
        &app.db,
        "SELECT definition||jsonb_build_object('payload',payload) FROM jobs WHERE id=$1",
        &[jid],
    )
    .await?;
    let root = app.root.join("jobs").join(jid);
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::write(
        root.join("event.json"),
        serde_json::to_vec_pretty(&r["payload"])?,
    )
    .await?;
    let mut env = super::runtime::environment()?;
    env.extend([
        ("RALLY_URL".into(), var("RALLY_WORKER_URL", &app.base)),
        ("RALLY_TOKEN".into(), token.into()),
        (
            "RALLY_EVENT".into(),
            if engine::local() {
                root.join("event.json").display().to_string()
            } else {
                "/workspace/event.json".into()
            },
        ),
    ]);
    let access = connections::Lease::new(app, connections::Scope::Job(jid.into()));
    env.extend(routing::worker_env(app, &access, &r["repos"]).await?);
    let name = format!("rally-{jid}");
    let mut shutdown = app.shutdown.clone();
    for repo in r["repos"].as_array().into_iter().flatten() {
        let origin = access.url(field(repo, "name")?);
        let args = [
            "clone",
            "--branch",
            field(repo, "branch")?,
            "--",
            &origin,
            field(repo, "name")?,
        ];
        let clone = engine::command(app, &root, None, &env, Some(&name), "git", &args)?;
        tokio::select! {
            result = engine::output(clone) => { result?; }
            _ = shutdown.changed() => {
                engine::stop(&name).await;
                return Ok(("aborted", String::new()));
            }
        }
    }
    let mut cmd = engine::command(
        app,
        &root,
        None,
        &env,
        Some(&name),
        "sh",
        &[
            "-c",
            r#"cd -- "$1" && exec sh -c "$2""#,
            "rally",
            directory(r["directory"].as_str())?,
            field(&r, "command")?,
        ],
    )?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let mut child = cmd.spawn()?;
    let pid = child.id().unwrap();
    let stdout = tokio::spawn(tail(child.stdout.take().unwrap()));
    let stderr = tokio::spawn(tail(child.stderr.take().unwrap()));
    let status = tokio::select! {
        result=child.wait()=>if result?.success(){"done"}else{"failed"},
        _=shutdown.changed()=>{
            if engine::local(){
                let _=tokio::process::Command::new("kill").args(["-TERM","--",&format!("-{pid}")]).output().await;
                if tokio::time::timeout(Duration::from_secs(10),child.wait()).await.is_err(){let _=tokio::process::Command::new("kill").args(["-KILL","--",&format!("-{pid}")]).output().await;}
            }else{engine::stop(&name).await;}
            "aborted"
        }
    };
    Ok((status, format!("{}{}", stdout.await??, stderr.await??)))
}

async fn tail(mut pipe: impl tokio::io::AsyncRead + Unpin) -> Result<String> {
    let mut buf = [0; 4096];
    let mut out = vec![];
    loop {
        let n = pipe.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > 32768 {
            out.drain(..out.len() - 32768);
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}
