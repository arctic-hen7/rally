use super::*;
use rally_teams::{digest, secret, slug};

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    let db = &app.db;
    if matches!(op, "member.remove" | "group.remove" | "account.role") {
        return change_membership(app, a, op, d).await;
    }
    match op {
        "topic.list"=>rows(db,"SELECT jsonb_build_object('slug',slug,'name',name) FROM topics ORDER BY name",&[]).await,
        "topic.create"=>{
            personal(a)?;let s=slug(field(d,"slug")?)?.to_lowercase();
            execute(db,"INSERT INTO topics(slug,name) VALUES($1,$2)",&[&s,field(d,"name")?]).await?;
            Ok(json!({"slug":s}))
        }
        "mission.create"|"mission.fork"=>{
            personal(a)?;
            let from=if op=="mission.fork"{Some(mission(app,a,field(d,"mission")?,false).await?)}else{None};
            let topic=if let Some(mid)=&from {mid.rsplit_once('-').unwrap().0}else{field(d,"topic")?};
            let title=field(d,"title")?;
            let repos=d["repos"].as_array();
            if d.get("repos").is_some() && repos.is_none(){bail!("Expected a repository list");}
            if from.is_some() && repos.is_some(){bail!("Take over forked repositories in the new mission");}
            let mut verified=Vec::new();
            for r in repos.into_iter().flatten(){
                let cid=git::verify_binding(app,a,r,field(r,"branch")?,r["connection"].as_str().filter(|v|!v.is_empty()),d["confirm"]==true || r["confirm"]==true).await?;
                verified.push((r,cid));
            }
            let mut tx=db.begin().await?;
            let n:i32=sqlx::query_scalar("UPDATE topics SET seq=seq+1 WHERE slug=$1 RETURNING seq").bind(topic).fetch_optional(&mut *tx).await?.ok_or_else(||anyhow::anyhow!("Unknown topic"))?;
            let mid=format!("{topic}-{n}");
            sqlx::query("INSERT INTO missions(id,topic,title,description) VALUES($1,$2,$3,$4)").bind(&mid).bind(topic).bind(title).bind(opt(d,"description")).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO members VALUES($1,$2)").bind(&mid).bind(&a.uid).execute(&mut *tx).await?;
            let tid=id("th");
            sqlx::query("INSERT INTO threads(id,mission,title,kind) VALUES($1,$2,'Main','main')").bind(&tid).bind(&mid).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO participants VALUES($1,$2)").bind(&tid).bind(&a.uid).execute(&mut *tx).await?;
            if let Some(parent)=&from{
                sqlx::query("INSERT INTO repos(mission,name,remote,branch,status) SELECT $1,name,remote,CASE WHEN NOT read_only THEN $2 END,CASE WHEN read_only THEN '{}'::jsonb ELSE jsonb_build_object('fork_from',branch) END FROM repos WHERE mission=$3").bind(&mid).bind(&mid).bind(parent).execute(&mut *tx).await?;
            }
            for (r,cid) in verified{
                sqlx::query("INSERT INTO repos(mission,name,remote,branch,contributor,connection,needs_transfer) VALUES($1,$2,$3,$4,$5,$6,false)").bind(&mid).bind(field(r,"name")?).bind(field(r,"remote")?).bind(field(r,"branch")?).bind(&a.uid).bind(cid).execute(&mut *tx).await?;
            }
            tx.commit().await?;Ok(json!({"id":mid,"thread":tid}))
        }
        "mission.list"=>{
            let all=rows(db,"SELECT to_jsonb(m) FROM missions m WHERE ($1='' OR topic=$1) AND ($2='' OR status=$2) AND (can_read($3,id) OR $4!='') ORDER BY created DESC",&[opt(d,"topic"),opt(d,"status"),&a.uid,a.run.as_deref().unwrap_or("")]).await?;
            let mut out=vec![];
            for item in all.as_array().unwrap(){if a.run.is_none() || mission(app,a,field(item,"id")?,false).await.is_ok(){out.push(item);}}
            Ok(json!(out))
        }
        "mission.show"|"mission.update"=>{
            let mid=mission(app,a,field(d,"mission")?,op!="mission.show").await?;
            if op!="mission.show"{personal(a)?;}
            if op=="mission.update" {sqlx::query("UPDATE missions SET title=COALESCE($2,title),description=COALESCE($3,description),status=COALESCE($4,status) WHERE id=$1").bind(&mid).bind(d["title"].as_str()).bind(d["description"].as_str()).bind(d["status"].as_str()).execute(db).await?;}
            let mut m=one(db,"SELECT to_jsonb(m) FROM missions m WHERE id=$1",&[&mid]).await?;
            m["repos"]=rows(db,"SELECT to_jsonb(r) FROM repo_details r WHERE mission=$1 AND NOT read_only ORDER BY name",&[&mid]).await?;
            m["read_only_repos"]=rows(db,"SELECT to_jsonb(r) FROM repo_details r WHERE mission=$1 AND read_only ORDER BY remote",&[&mid]).await?;
            m["git_requests"]=routing::pending(app,&mid).await?;
            m["threads"]=rows(db,"SELECT to_jsonb(t) FROM threads t WHERE mission=$1 ORDER BY (kind='main') DESC,created DESC",&[&mid]).await?;
            Ok(m)
        }
        "people.list"=>rows(db,"SELECT jsonb_build_object('id',id,'name',name,'kind',kind,'owner',owner,'admin',admin) FROM people ORDER BY name",&[]).await,
        "agent.list" =>rows(db,"SELECT to_jsonb(a)||jsonb_build_object('introduce',can_introduce($1,id)) FROM agents a WHERE can_discover($1,id) ORDER BY name",&[&a.uid]).await,
        "agent.show"=>one(db,"SELECT to_jsonb(a) FROM agents a WHERE (id=$1 OR name=$1) AND can_discover($2,id)",&[field(d,"agent")?,&a.uid]).await,
        "agent.create"=>{
            personal(a)?;let aid=id("a");
            execute(db,"INSERT INTO agents(id,name,instructions,model,creator,private) VALUES($1,$2,$3,$4,$5,$6::text::boolean)",&[&aid,slug(field(d,"name")?)?,opt(d,"instructions"),field(d,"model")?,&a.uid,if d["private"]==true{"true"}else{"false"}]).await?;Ok(json!({"id":aid}))
        }
        "agent.update"=>{
            personal(a)?;
            let aid:String=sqlx::query_scalar("UPDATE agents SET instructions=coalesce($2,instructions),model=coalesce($3,model) WHERE (id=$1 OR name=$1) AND (creator=$4 OR $5) RETURNING id").bind(field(d,"agent")?).bind(d["instructions"].as_str()).bind(d["model"].as_str()).bind(&a.uid).bind(a.owner || a.admin).fetch_optional(db).await?.ok_or_else(||anyhow::anyhow!("Access denied"))?;
            Ok(json!({"id":aid}))
        }
        "member.list"=>{
            let mid=mission(app,a,field(d,"mission")?,false).await?;
            rows(db,"SELECT jsonb_build_object('id',principal,'name',coalesce(p.name,g.name,a.name),'kind',coalesce(p.kind,CASE WHEN g.id IS NULL THEN 'agent' ELSE 'group' END)) FROM members m LEFT JOIN people p ON p.id=principal LEFT JOIN groups g ON g.id=principal LEFT JOIN agents a ON a.id=principal WHERE mission=$1 ORDER BY principal",&[&mid]).await
        }
        "member.add"=>{
            personal(a)?;let mid=mission(app,a,field(d,"mission")?,true).await?;
            let pid=field(d,"principal")?;
            {
                let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT id FROM people WHERE id=$1 UNION ALL SELECT id FROM groups WHERE id=$1 UNION ALL SELECT id FROM agents WHERE id=$1)").bind(pid).fetch_one(db).await?;
                if !exists{bail!("Unknown principal");}
                if pid.starts_with("a_"){
                    let private:Option<bool>=sqlx::query_scalar("SELECT private FROM agents WHERE id=$1 AND can_introduce($2,id)").bind(pid).bind(&a.uid).fetch_optional(db).await?;
                    if private.is_none(){bail!("Access denied");}
                    if private==Some(true) && d["confirm"]!=true{bail!("Confirm that mission members can use this private agent and any secrets the agent has");}
                }
                execute(db,"INSERT INTO members VALUES($1,$2) ON CONFLICT DO NOTHING",&[&mid,pid]).await?;
            }
            Ok(json!({"id":pid}))
        }
        "status.list" | "status.create" | "status.update" | "status.remove" => status(db, a, op, d).await,
        "group.list"=>rows(db,"SELECT to_jsonb(g)||jsonb_build_object('members',ARRAY(SELECT person FROM group_members WHERE group_id=g.id)) FROM groups g ORDER BY name",&[]).await,
        "group.create"=>{
            admin(a)?;let gid=id("g");execute(db,"INSERT INTO groups VALUES($1,$2)",&[&gid,field(d,"name")?]).await?;Ok(json!({"id":gid}))
        }
        "group.add"=>{
            admin(a)?;let sql="INSERT INTO group_members VALUES($1,$2) ON CONFLICT DO NOTHING";
            execute(db,sql,&[field(d,"group")?,field(d,"person")?]).await?;Ok(json!({"ok":true}))
        }
        "account.invite"=>{
            owner_access(a)?;
            let code=secret();
            sqlx::query("INSERT INTO invites(digest,email,expires,admin) VALUES($1,$2,now()+interval '7 days',$3)").bind(digest(&code)).bind(field(d,"email")?).bind(role(d)?).execute(db).await?;Ok(json!({"code":code}))
        }
        "account.service"=>{
            admin(a)?;let uid=id("u");let token=secret();let mut tx=db.begin().await?;
            sqlx::query("INSERT INTO people(id,name,kind) VALUES($1,$2,'service')").bind(&uid).bind(field(d,"name")?).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO tokens(digest,person,label) VALUES($1,$2,'Service token')").bind(digest(&token)).bind(&uid).execute(&mut *tx).await?;
            tx.commit().await?;Ok(json!({"id":uid,"token":token}))
        }
        "token.create"=>{
            if a.run.is_some() || a.routine.is_some(){bail!("Access denied");}
            let token=secret();execute(db,"INSERT INTO tokens(digest,person,label) VALUES($1,$2,$3)",&[&digest(&token),&a.uid,field(d,"name")?]).await?;Ok(json!({"token":token}))
        }
        "token.list"|"token.revoke"=>{
            if a.run.is_some() || a.routine.is_some(){bail!("Access denied");}
            if op=="token.revoke"{sqlx::query("DELETE FROM tokens WHERE digest=$1 AND (person=$2 OR $3)").bind(field(d,"id")?).bind(&a.uid).bind(a.owner).execute(db).await?;}
            rows(db,"SELECT jsonb_build_object('id',digest,'label',label,'created',created) FROM tokens WHERE person=$1 ORDER BY created DESC",&[&a.uid]).await
        }
        _=>bail!("Unknown operation")
    }
}

async fn status(db: &PgPool, actor: &Actor, op: &str, data: &Value) -> Result<Value> {
    if op == "status.list" {
        return rows(
            db,
            "SELECT to_jsonb(s) FROM mission_statuses s ORDER BY position,name",
            &[],
        )
        .await;
    }
    admin(actor)?;
    if op == "status.remove" {
        let status_id = field(data, "status")?;
        if status_id == "open" {
            bail!("The initial mission status is required");
        }
        let result = sqlx::query(
            "DELETE FROM mission_statuses WHERE id=$1 AND NOT EXISTS(SELECT 1 FROM missions WHERE status=$1)"
        ).bind(status_id).execute(db).await?;
        if result.rows_affected() == 0 {
            bail!("Choose an existing status that has no missions");
        }
        return Ok(json!({"ok":true}));
    }
    let color = field(data, "color")?;
    if !["slate", "blue", "teal", "amber", "violet", "rose", "green"].contains(&color) {
        bail!("Choose a status color");
    }
    let name = field(data, "name")?.trim();
    if op == "status.create" {
        let status_id = id("st");
        execute(db, "INSERT INTO mission_statuses(id,name,color,position) SELECT $1,$2,$3,COALESCE(MAX(position),0)+1 FROM mission_statuses", &[&status_id, name, color]).await?;
        Ok(json!({"id":status_id}))
    } else {
        let status_id = field(data, "status")?;
        one(
            db,
            "UPDATE mission_statuses SET name=$2,color=$3 WHERE id=$1 RETURNING to_jsonb(id)",
            &[status_id, name, color],
        )
        .await?;
        Ok(json!({"id":status_id}))
    }
}

fn owner_access(a: &Actor) -> Result<()> {
    admin(a)?;
    if !a.owner {
        bail!("Access denied");
    }
    Ok(())
}

fn role(d: &Value) -> Result<bool> {
    match d["role"].as_str().unwrap_or("member") {
        "member" => Ok(false),
        "admin" => Ok(true),
        _ => bail!("Choose member or admin"),
    }
}

async fn change_membership(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    let (mid, person) = match op {
        "member.remove" => (
            mission(app, a, field(d, "mission")?, true).await?,
            field(d, "principal")?,
        ),
        "group.remove" => {
            admin(a)?;
            (String::new(), field(d, "person")?)
        }
        _ => {
            owner_access(a)?;
            (String::new(), field(d, "person")?)
        }
    };
    let mut tx = app.db.begin().await?;
    // Keep the preview and the membership mutation on a consistent set of repository grants
    sqlx::query("LOCK TABLE members,group_members,people IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let candidates:Vec<(String,String)>=sqlx::query_as("SELECT mission,name FROM repos WHERE NOT needs_transfer AND ($1='' OR mission=$1) AND (contributor=$2 OR contributor IN (SELECT person FROM group_members WHERE group_id=$2)) ORDER BY mission,name FOR UPDATE")
        .bind(&mid).bind(person).fetch_all(&mut *tx).await?;
    match op {
        "member.remove" => {
            sqlx::query("DELETE FROM members WHERE mission=$1 AND principal=$2")
                .bind(&mid)
                .bind(person)
                .execute(&mut *tx)
                .await?;
        }
        "group.remove" => {
            sqlx::query("DELETE FROM group_members WHERE group_id=$1 AND person=$2")
                .bind(field(d, "group")?)
                .bind(person)
                .execute(&mut *tx)
                .await?;
        }
        _ => {
            field(d, "role")?;
            let changed =
                sqlx::query("UPDATE people SET admin=$2 WHERE id=$1 AND NOT owner AND kind='user'")
                    .bind(person)
                    .bind(role(d)?)
                    .execute(&mut *tx)
                    .await?;
            if changed.rows_affected() != 1 {
                bail!("Choose a regular user account");
            }
        }
    }
    let (missions, names): (Vec<_>, Vec<_>) = candidates.into_iter().unzip();
    let affected:Vec<String>=sqlx::query_scalar("SELECT r.mission||' / '||CASE WHEN r.read_only THEN r.remote ELSE r.name END FROM repos r JOIN unnest($1::text[],$2::text[]) c(mission,name) USING(mission,name) WHERE r.needs_transfer ORDER BY r.mission,r.name").bind(missions).bind(names).fetch_all(&mut *tx).await?;
    if d["preview"] == true {
        tx.rollback().await?;
        return Ok(json!({"affected":affected}));
    }
    if !affected.is_empty() && d["confirm"] != true {
        bail!(
            "These repositories will need transfer: {}. Confirm removal to pause their remote access.",
            affected.join(", ")
        );
    }
    let remaining:Vec<String>=sqlx::query_scalar("SELECT name FROM groups g JOIN group_members gm ON gm.group_id=g.id JOIN members m ON m.principal=g.id WHERE gm.person=$1 AND m.mission=$2 UNION ALL SELECT 'workspace administrator' FROM people WHERE id=$1 AND (owner OR admin)").bind(person).bind(&mid).fetch_all(&mut *tx).await?;
    let retained:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('name',p.name,'id',p.id) FROM group_members gm JOIN people p ON p.id=gm.person WHERE gm.group_id=$1 AND can_read(p.id,$2) ORDER BY p.name").bind(person).bind(&mid).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(json!({"id":person,"remaining":remaining,"retained":retained,"affected":affected}))
}
