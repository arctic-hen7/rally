use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use rally_teams::{remote, slug};
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
async fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.ext.allow=never",
        ])
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
async fn load(app: &App, s: &Value) -> Result<()> {
    let sid = field(s, "id")?;
    let url = remote(field(s, "remote")?)?;
    let dir = app.root.join("library").join(sid);
    tokio::fs::create_dir_all(&dir).await?;
    if !dir.join("HEAD").exists() {
        git(&dir, &["init", "--bare", "."]).await?;
    }
    let branch = s["ref"].as_str().unwrap_or("HEAD");
    if branch.starts_with('-') {
        bail!("Invalid skill ref");
    }
    git(&dir, &["fetch", "--depth=1", "--", url, branch]).await?;
    let rev = String::from_utf8(git(&dir, &["rev-parse", "FETCH_HEAD"]).await?)?
        .trim()
        .to_string();
    if s["revision"] == rev {
        return Ok(());
    }
    let sub = opt(s, "path");
    let tree = if sub.is_empty() {
        rev.clone()
    } else {
        format!("{rev}:{}", relative(sub)?)
    };
    let listing = String::from_utf8(git(&dir, &["ls-tree", "-r", "-z", &tree]).await?)?;
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
        let n: usize = String::from_utf8(git(&dir, &["cat-file", "-s", object]).await?)?
            .trim()
            .parse()?;
        size += n;
        if size > 8 * 1024 * 1024 {
            bail!("Skill bundle exceeds 8 MiB");
        }
        let data = git(&dir, &["cat-file", "blob", object]).await?;
        if path == "SKILL.md" {
            body = String::from_utf8(data.clone())?;
        }
        bundle[path] = json!({"data":STANDARD.encode(data),"executable":mode=="100755"});
    }
    if body.is_empty() {
        bail!("The selected Git directory requires SKILL.md");
    }
    sqlx::query("UPDATE skills SET body=$2,revision=$3,files=$4,sync_error=NULL WHERE id=$1")
        .bind(sid)
        .bind(body)
        .bind(rev)
        .bind(bundle)
        .execute(&app.db)
        .await?;
    Ok(())
}

pub async fn refresh(app: &App) -> Result<()> {
    for s in rows(
        &app.db,
        "SELECT to_jsonb(s)-'files' FROM skills s WHERE remote IS NOT NULL",
        &[],
    )
    .await?
    .as_array()
    .unwrap()
    {
        if let Err(e) = load(app, s).await {
            execute(
                &app.db,
                "UPDATE skills SET sync_error=$2 WHERE id=$1",
                &[field(s, "id")?, &e.to_string()],
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    match op{
        "skill.list"=>rows(&app.db,"SELECT (to_jsonb(s)-'files'-'body')||jsonb_build_object('targets',ARRAY(SELECT target FROM attachments WHERE skill=s.id)) FROM skills s ORDER BY name",&[]).await,
        "skill.show"=>one(&app.db,"SELECT to_jsonb(s)-'files' FROM skills s WHERE id=$1 OR name=$1",&[field(d,"skill")?]).await,
        "skill.bundle"=>{
            let mid=mission(app,a,field(d,"mission")?,false).await?;
            bundle(app,&mid,"").await
        }
        "skill.create"|"skill.import"=>{
            personal(a)?;let name=slug(field(d,"name")?)?;if name=="rally"{bail!("The rally skill is built in");}
            let sid=id("sk");
            if op=="skill.import"{remote(field(d,"remote")?)?;}else{field(d,"body")?;}
            sqlx::query("INSERT INTO skills(id,name,body,remote,ref,path,creator) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(&sid).bind(name).bind(opt(d,"body")).bind(d["remote"].as_str()).bind(d["ref"].as_str()).bind(d["path"].as_str()).bind(&a.uid).execute(&app.db).await?;
            if op=="skill.import"{let s=one(&app.db,"SELECT to_jsonb(s)-'files' FROM skills s WHERE id=$1",&[&sid]).await?;if let Err(e)=load(app,&s).await{execute(&app.db,"UPDATE skills SET sync_error=$2 WHERE id=$1",&[&sid,&e.to_string()]).await?;return Err(e);}}
            Ok(json!({"id":sid}))
        }
        "skill.edit"=>{
            personal(a)?;
            let result=sqlx::query("UPDATE skills SET body=$2 WHERE (id=$1 OR name=$1) AND remote IS NULL AND (creator=$3 OR $4)").bind(field(d,"skill")?).bind(field(d,"body")?).bind(&a.uid).bind(a.owner).execute(&app.db).await?;
            if result.rows_affected()==0{bail!("Edit imported skills in Git, or check ownership");}
            Ok(json!({"ok":true}))
        }
        "skill.attach"|"skill.detach"=>{
            personal(a)?;
            let input=field(d,"target")?.trim_start_matches('#');
            let target=if let Some(agent)=sqlx::query("SELECT id,creator FROM agents WHERE id=$1 OR name=$1").bind(input.trim_start_matches('@')).fetch_optional(&app.db).await?{
                if !a.owner && agent.get::<String,_>("creator")!=a.uid{bail!("Access denied");}agent.get("id")
            }else{mission(app,a,input,true).await?};
            let sid:String=sqlx::query_scalar("SELECT id FROM skills WHERE id=$1 OR name=$1").bind(field(d,"skill")?).fetch_one(&app.db).await?;
            execute(&app.db,if op=="skill.attach"{"INSERT INTO attachments VALUES($1,$2) ON CONFLICT DO NOTHING"}else{"DELETE FROM attachments WHERE skill=$1 AND target=$2"},&[&sid,&target]).await?;
            Ok(json!({"ok":true}))
        }
        _=>bail!("Unknown skill operation")
    }
}

pub async fn bundle(app: &App, mid: &str, agent: &str) -> Result<Value> {
    let mut result =
        json!({"rally":{"SKILL.md":{"data":STANDARD.encode(RALLY_SKILL),"executable":false}}});
    for row in sqlx::query("SELECT DISTINCT s.id,s.name,s.body,s.files FROM skills s JOIN attachments t ON t.skill=s.id WHERE t.target=$1 OR t.target=$2").bind(mid).bind(agent).fetch_all(&app.db).await?{
        let name:&str=row.get("name");let mut files:Value=row.get("files");
        files["SKILL.md"]=json!({"data":STANDARD.encode(row.get::<String,_>("body")),"executable":false});
        result[name]=files;
    }
    Ok(result)
}
