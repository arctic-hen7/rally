use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use rally_teams::slug;
use sqlx::{Postgres, Transaction};
use std::path::{Component, Path};
use tokio::process::Command;

pub const RALLY_SKILL: &str = include_str!("../../runtime/skills/rally/SKILL.md");

pub fn relative(s: &str) -> Result<&str> {
    if s.is_empty()
        || !Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        bail!("Invalid skill path");
    }
    Ok(s)
}
async fn git(dir: &Path, env: &[(String, String)], args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.ext.allow=never",
        ])
        .envs(env.iter().cloned())
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "Git skill fetch failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}
async fn load(app: &App, sid: &str, reconnect: Option<(&str, Option<&str>)>) -> Result<()> {
    let _guard = app.skill_lock.lock().await;
    let mut tx = app.db.begin().await?;
    if let Some((owner, Some(connection))) = reconnect {
        sqlx::query("SELECT id FROM git_connections WHERE id=$1 AND owner_id=$2 FOR KEY SHARE")
            .bind(connection)
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
    }
    let s: Value = sqlx::query_scalar(
        "SELECT to_jsonb(s)-'files' FROM skills s WHERE id=$1 AND remote IS NOT NULL FOR UPDATE",
    )
    .bind(sid)
    .fetch_one(&mut *tx)
    .await?;
    if s["git_paused"] == true && reconnect.is_none() {
        return Ok(());
    }
    if let Some((owner, connection)) = reconnect {
        sqlx::query("UPDATE skills SET git_owner=$2,connection=$3,git_paused=false WHERE id=$1")
            .bind(sid)
            .bind(owner)
            .bind(connection)
            .execute(&mut *tx)
            .await?;
    }
    let (owner, connection) = reconnect.unwrap_or((
        s["git_owner"].as_str().unwrap_or(field(&s, "creator")?),
        s["connection"].as_str(),
    ));
    let result = fetch_snapshot(app, &mut tx, &s, owner, connection).await;
    if let Err(error) = &result {
        if reconnect.is_some() {
            return result;
        }
        let details = error.to_string();
        let paused =
            details.contains("returned error: 403") || details.contains("returned error: 404");
        sqlx::query("UPDATE skills SET sync_error=$2,git_paused=$3 WHERE id=$1")
            .bind(sid)
            .bind(details)
            .bind(paused)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    result
}

async fn fetch_snapshot(
    app: &App,
    tx: &mut Transaction<'_, Postgres>,
    s: &Value,
    owner: &str,
    connection: Option<&str>,
) -> Result<()> {
    let sid = field(s, "id")?;
    let url = field(s, "remote")?;
    let dir = app.root.join("library").join(sid);
    tokio::fs::create_dir_all(&dir).await?;
    if !dir.join("HEAD").exists() {
        git(&dir, &[], &["init", "--bare", "."]).await?;
    }
    let branch = s["ref"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or("HEAD");
    if branch.starts_with('-') {
        bail!("Invalid skill ref");
    }
    let lease = connections::source(app, url, owner, connection);
    git(
        &dir,
        &lease.env(),
        &["fetch", "--depth=1", "--", &lease.url("repo"), branch],
    )
    .await?;
    let rev = String::from_utf8(git(&dir, &[], &["rev-parse", "FETCH_HEAD"]).await?)?
        .trim()
        .to_string();
    if s["revision"] == rev {
        sqlx::query("UPDATE skills SET sync_error=NULL,last_fetch=now() WHERE id=$1")
            .bind(sid)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }
    let sub = opt(s, "path");
    let tree = if sub.is_empty() {
        rev.clone()
    } else {
        format!("{rev}:{}", relative(sub)?)
    };
    let listing = String::from_utf8(git(&dir, &[], &["ls-tree", "-r", "-z", &tree]).await?)?;
    let mut bundle = json!({});
    let mut body = String::new();
    let mut size = 0_usize;
    for row in listing.split('\0').filter(|s| !s.is_empty()) {
        let (meta, path) = row
            .split_once('\t')
            .ok_or_else(|| anyhow::anyhow!("Malformed Git tree"))?;
        relative(path)?;
        let mut parts = meta.split_whitespace();
        let mode = parts.next().unwrap_or("");
        if !["100644", "100755"].contains(&mode) {
            continue;
        }
        let object = meta.split_whitespace().last().unwrap();
        let n: usize = String::from_utf8(git(&dir, &[], &["cat-file", "-s", object]).await?)?
            .trim()
            .parse()?;
        size += n;
        if size > 8 * 1024 * 1024 {
            bail!("Skill bundle exceeds 8 MiB");
        }
        let data = git(&dir, &[], &["cat-file", "blob", object]).await?;
        if path == "SKILL.md" {
            body = String::from_utf8(data.clone())?;
        }
        bundle[path] = json!({"data":STANDARD.encode(data),"executable":mode=="100755"});
    }
    if body.is_empty() {
        bail!("The selected Git directory requires SKILL.md");
    }
    let description = body
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---"))
        .and_then(|(meta, _)| serde_yaml_ng::from_str::<Value>(meta).ok())
        .and_then(|v| v["description"].as_str().map(str::to_owned))
        .unwrap_or_default();
    sqlx::query(
        "UPDATE skills SET body=$2,revision=$3,files=$4,description=$5,sync_error=NULL,last_fetch=now() WHERE id=$1 AND NOT git_paused",
    )
    .bind(sid)
    .bind(body)
    .bind(rev)
    .bind(bundle)
    .bind(description)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn refresh(app: &App) -> Result<()> {
    let imports: Vec<String> =
        sqlx::query_scalar("SELECT id FROM skills WHERE remote IS NOT NULL AND NOT git_paused")
            .fetch_all(&app.db)
            .await?;
    for sid in imports {
        if let Err(error) = load(app, &sid, None).await {
            eprintln!("Skill {sid}: {error}");
        }
    }
    Ok(())
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    match op{
        "skill.list"=>rows(&app.db,"SELECT to_jsonb(s)-'files'-'body' FROM skills s WHERE can_discover($1,id) ORDER BY name",&[&a.uid]).await,
        "skill.show"=>one(&app.db,"SELECT to_jsonb(s)-'files' FROM skills s WHERE (id=$1 OR name=$1) AND can_discover($2,id)",&[field(d,"skill")?,&a.uid]).await,
        "skill.bundle"=>{
            let mid=mission(app,a,field(d,"mission")?,false).await?;
            bundle(app,&mid,"").await
        }
        "skill.create"|"skill.import"=>{
            personal(a)?;let name=slug(field(d,"name")?)?;if name=="rally"{bail!("The rally skill is built in");}
            let sid=id("sk");
            let cid=if op=="skill.import"{
                let url=field(d,"remote")?;
                let cid=connections::selected(app,a,url,d["connection"].as_str().filter(|s|!s.is_empty())).await?;
                connections::validate(app,&a.uid,url,cid.as_deref()).await?;
                cid
            }else{field(d,"body")?;field(d,"description")?;if d.get("remote").is_some(){bail!("Use skill import for a Git source");}None};
            let _guard=if op=="skill.import"{Some(app.skill_lock.lock().await)}else{None};
            let mut tx=app.db.begin().await?;
            let source: Value=sqlx::query_scalar("INSERT INTO skills AS s(id,name,body,remote,ref,path,creator,description,private,git_owner,connection) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$7,$10) RETURNING to_jsonb(s)-'files'")
                .bind(&sid).bind(name).bind(opt(d,"body")).bind(d["remote"].as_str()).bind(d["ref"].as_str()).bind(d["path"].as_str()).bind(&a.uid).bind(opt(d,"description")).bind(d["private"]==true).bind(&cid).fetch_one(&mut *tx).await?;
            if op=="skill.import" && let Err(error)=fetch_snapshot(app,&mut tx,&source,&a.uid,cid.as_deref()).await{
                tx.rollback().await?;
                let _=tokio::fs::remove_dir_all(app.root.join("library").join(&sid)).await;
                return Err(error);
            }
            tx.commit().await?;
            Ok(json!({"id":sid}))
        }
        "skill.reconnect"=>{
            let sid=field(d,"skill")?;
            access::manager(app,a,sid).await?;
            let s=one(&app.db,"SELECT to_jsonb(s)-'files' FROM skills s WHERE id=$1 AND remote IS NOT NULL",&[sid]).await?;
            let cid=d["connection"].as_str().filter(|s|!s.is_empty());
            let remote=field(&s,"remote")?;
            let choice=connections::selected(app,a,remote,cid).await?;
            let cid=choice.as_deref();
            connections::validate(app,&a.uid,remote,cid).await?;
            load(app,sid,Some((&a.uid,cid))).await?;
            Ok(json!({"id":sid}))
        }
        "skill.edit"=>{
            personal(a)?;
            let name=d["name"].as_str().map(slug).transpose()?;
            if d.get("description").is_some(){field(d,"description")?;}
            if name==Some("rally"){bail!("The rally skill is built in");}
            let result=sqlx::query("UPDATE skills SET body=$2,name=COALESCE($5,name),description=COALESCE($6,description) WHERE (id=$1 OR name=$1) AND remote IS NULL AND (creator=$3 OR $4)").bind(field(d,"skill")?).bind(field(d,"body")?).bind(&a.uid).bind(a.owner || a.admin).bind(name).bind(d["description"].as_str()).execute(&app.db).await?;
            if result.rows_affected()==0{bail!("Edit imported skills in Git, or check ownership");}
            Ok(json!({"ok":true}))
        }
        "skill.attach"|"skill.detach"=>{
            personal(a)?;
            let input=field(d,"target")?.trim_start_matches('#');
            let target=if let Some(agent)=sqlx::query("SELECT id,creator FROM agents WHERE id=$1 OR name=$1").bind(input.trim_start_matches('@')).fetch_optional(&app.db).await?{
                if !(a.owner || a.admin) && agent.get::<String,_>("creator")!=a.uid{bail!("Access denied");}agent.get("id")
            }else{mission(app,a,input,true).await?};
            let sid:String=sqlx::query_scalar("SELECT id FROM skills WHERE id=$1 OR name=$1").bind(field(d,"skill")?).fetch_one(&app.db).await?;
            if op=="skill.attach"{
                let private:Option<bool>=sqlx::query_scalar("SELECT private FROM skills WHERE id=$1 AND can_introduce($2,id)").bind(&sid).bind(&a.uid).fetch_optional(&app.db).await?;
                if private.is_none(){bail!("Access denied");}
                if private==Some(true) && d["confirm"]!=true {bail!("Confirm that everyone with access to the target can use this private skill and any secrets it has");}
            }
            execute(&app.db,if op=="skill.attach"{"INSERT INTO attachments VALUES($1,$2) ON CONFLICT DO NOTHING"}else{"DELETE FROM attachments WHERE skill=$1 AND target=$2"},&[&sid,&target]).await?;
            Ok(json!({"ok":true}))
        }
        _=>bail!("Unknown skill operation")
    }
}

pub async fn bundle(app: &App, mid: &str, agent: &str) -> Result<Value> {
    let mut result =
        json!({"rally":{"SKILL.md":{"data":STANDARD.encode(RALLY_SKILL),"executable":false}}});
    for row in sqlx::query("SELECT DISTINCT s.id,s.name,s.body,s.files,s.remote,s.description FROM skills s JOIN attachments t ON t.skill=s.id WHERE t.target=$1 OR t.target=$2").bind(mid).bind(agent).fetch_all(&app.db).await?{
        let name:&str=row.get("name");let mut files:Value=row.get("files");
        let mut body:String=row.get("body");
        let description:&str=row.get("description");
        if row.get::<Option<String>,_>("remote").is_none() && !description.is_empty() {
            let meta=serde_yaml_ng::to_string(&json!({"name":name,"description":description}))?;
            body=format!("---\n{meta}---\n\n{body}");
        }
        files["SKILL.md"]=json!({"data":STANDARD.encode(body),"executable":false});
        result[name]=files;
    }
    Ok(result)
}
