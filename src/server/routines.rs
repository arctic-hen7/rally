use super::*;
use super::{engine, git, notify, search, skills};
use chrono::{DateTime, Utc};
use cron::Schedule;
use rally_teams::{digest, remote, secret, slug};
use std::{process::Stdio, str::FromStr, time::Duration};
use tokio::io::AsyncReadExt;

fn next(expr: &str) -> Result<Option<DateTime<Utc>>> {
    if expr.is_empty() {
        return Ok(None);
    }
    let expr = if expr.split_whitespace().count() == 5 {
        format!("0 {expr}")
    } else {
        expr.into()
    };
    Ok(Some(
        Schedule::from_str(&expr)?
            .upcoming(Utc)
            .next()
            .ok_or_else(|| anyhow::anyhow!("Schedule has no future occurrence"))?,
    ))
}
fn validate_repos(repos: &Value) -> Result<()> {
    for r in repos
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Repos must be an array"))?
    {
        slug(field(r, "name")?)?;
        remote(field(r, "remote")?)?;
        if field(r, "branch")?.starts_with('-') {
            bail!("Invalid branch");
        }
    }
    Ok(())
}

async fn enqueue(app: &App, id: &str, payload: &Value) -> Result<Value> {
    let job = rally_teams::id("job");
    let n=sqlx::query("INSERT INTO jobs(id,routine,owner_id,payload,status) SELECT $1,id,owner_id,$3,'queued' FROM routines WHERE id=$2 AND enabled").bind(&job).bind(id).bind(payload).execute(&app.db).await?.rows_affected();
    if n != 1 {
        bail!("Routine disabled");
    }
    Ok(json!({"id":job}))
}
pub async fn webhook(
    State(app): State<App>,
    Path((rid, key)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> ApiResult {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM routines WHERE id=$1 AND hook_hash=$2 AND enabled)",
    )
    .bind(&rid)
    .bind(digest(&key))
    .fetch_one(&app.db)
    .await?;
    if !valid {
        return Err(anyhow::anyhow!("Not found").into());
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(enqueue(&app, &rid, &payload).await?),
    )
        .into_response())
}
pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    if op == "routine.list" {
        return rows(
            &app.db,
            "SELECT to_jsonb(r)-'hook_hash' FROM routines r WHERE owner_id=$1 ORDER BY name",
            &[&a.uid],
        )
        .await;
    }
    if op == "routine.create" {
        let rid = id("rt");
        let key = secret();
        let cron = opt(d, "cron");
        let repos = d.get("repos").cloned().unwrap_or(json!([]));
        validate_repos(&repos)?;
        sqlx::query("INSERT INTO routines(id,name,owner_id,command,cron,hook_hash,repos,next_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(&rid).bind(field(d,"name")?).bind(&a.uid).bind(field(d,"command")?).bind(cron).bind(digest(&key)).bind(repos).bind(next(cron)?).execute(&app.db).await?;
        return Ok(json!({"id":rid,"webhook":format!("{}/hooks/{rid}/{key}",app.base)}));
    }
    let rid = field(d, "routine")?;
    if op == "routine.transfer" {
        admin(a)?;
        execute(
            &app.db,
            "UPDATE routines SET owner_id=$2 WHERE id=$1",
            &[rid, field(d, "person")?],
        )
        .await?;
        return Ok(json!({"id":rid}));
    }
    let mut r = one(
        &app.db,
        "SELECT to_jsonb(r)-'hook_hash' FROM routines r WHERE id=$1 AND owner_id=$2",
        &[rid, &a.uid],
    )
    .await?;
    match op {
        "routine.show" => {
            r["jobs"]=rows(&app.db,"SELECT to_jsonb(j)-'token_digest'-'payload' FROM jobs j WHERE routine=$1 ORDER BY created DESC LIMIT 20",&[rid]).await?;
            Ok(r)
        }
        "routine.trigger" => enqueue(app, rid, &d["payload"]).await,
        "routine.update" => {
            let repos = d.get("repos").unwrap_or(&r["repos"]);
            validate_repos(repos)?;
            let expr = d["cron"].as_str().unwrap_or(opt(&r, "cron"));
            sqlx::query("UPDATE routines SET name=coalesce($2,name),command=coalesce($3,command),cron=$4,next_at=$5,repos=$6 WHERE id=$1").bind(rid).bind(d["name"].as_str()).bind(d["command"].as_str()).bind(expr).bind(next(expr)?).bind(repos).execute(&app.db).await?;
            Ok(json!({"id":rid}))
        }
        "routine.enable" | "routine.disable" => {
            sqlx::query("UPDATE routines SET enabled=$2,next_at=$3 WHERE id=$1")
                .bind(rid)
                .bind(op == "routine.enable")
                .bind(next(opt(&r, "cron"))?)
                .execute(&app.db)
                .await?;
            Ok(json!({"id":rid}))
        }
        _ => bail!("Unknown routine operation"),
    }
}

async fn schedule(app: &App) -> Result<()> {
    let mut tx = app.db.begin().await?;
    for r in sqlx::query("SELECT id,cron,owner_id FROM routines WHERE enabled AND next_at<=now() FOR UPDATE SKIP LOCKED").fetch_all(&mut *tx).await?{
        sqlx::query("INSERT INTO jobs(id,routine,owner_id,payload,status) VALUES($1,$2,$3,'{}','queued')").bind(id("job")).bind(r.get::<String,_>("id")).bind(r.get::<String,_>("owner_id")).execute(&mut *tx).await?;
        sqlx::query("UPDATE routines SET next_at=$2 WHERE id=$1").bind(r.get::<String,_>("id")).bind(next(r.get("cron"))?).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
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
                schedule(&app).await?;
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
        if cycle % 5 == 0 {
            if let Err(e) = notify::deliver(&app).await {
                eprintln!("Push: {e}");
            }
        }
        if cycle % 30 == 0 {
            if let Err(e) = skills::refresh(&app).await {
                eprintln!("Skills: {e}");
            }
            if let Ok(missions) =
                sqlx::query_scalar::<_, String>("SELECT DISTINCT mission FROM repos")
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
    let r=one(&app.db,"SELECT (to_jsonb(r)-'hook_hash')||jsonb_build_object('payload',j.payload) FROM jobs j JOIN routines r ON r.id=j.routine WHERE j.id=$1",&[jid]).await?;
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
        ("RALLY_EVENT".into(), "event.json".into()),
    ]);
    for repo in r["repos"].as_array().into_iter().flatten() {
        let args = [
            "clone",
            "--branch",
            field(repo, "branch")?,
            "--",
            field(repo, "remote")?,
            field(repo, "name")?,
        ];
        engine::output(engine::command(app, &root, None, &env, None, "git", &args)?).await?;
    }
    let name = format!("rally-{jid}");
    let mut cmd = engine::command(
        app,
        &root,
        None,
        &env,
        Some(&name),
        "sh",
        &["-c", field(&r, "command")?],
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
    let mut shutdown = app.shutdown.clone();
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
