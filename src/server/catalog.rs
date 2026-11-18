use super::*;
use rally_teams::{digest, secret, slug};

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    let db = &app.db;
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
            let mut tx=db.begin().await?;
            let n:i32=sqlx::query_scalar("UPDATE topics SET seq=seq+1 WHERE slug=$1 RETURNING seq").bind(topic).fetch_optional(&mut *tx).await?.ok_or_else(||anyhow::anyhow!("Unknown topic"))?;
            let mid=format!("{topic}-{n}");
            sqlx::query("INSERT INTO missions(id,topic,title,description) VALUES($1,$2,$3,$4)").bind(&mid).bind(topic).bind(title).bind(opt(d,"description")).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO members VALUES($1,$2)").bind(&mid).bind(&a.uid).execute(&mut *tx).await?;
            let tid=id("th");
            sqlx::query("INSERT INTO threads(id,mission,title,kind) VALUES($1,$2,'Main','main')").bind(&tid).bind(&mid).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO participants VALUES($1,$2)").bind(&tid).bind(&a.uid).execute(&mut *tx).await?;
            if let Some(parent)=&from{
                sqlx::query("INSERT INTO repos(mission,name,remote,branch,status) SELECT $1,name,remote,$2,jsonb_build_object('fork_from',branch) FROM repos WHERE mission=$3").bind(&mid).bind(&mid).bind(parent).execute(&mut *tx).await?;
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
            m["repos"]=rows(db,"SELECT to_jsonb(r) FROM repos r WHERE mission=$1 ORDER BY name",&[&mid]).await?;
            m["threads"]=rows(db,"SELECT to_jsonb(t) FROM threads t WHERE mission=$1 ORDER BY (kind='main') DESC,created DESC",&[&mid]).await?;
            Ok(m)
        }
        "people.list"=>rows(db,"SELECT jsonb_build_object('id',id,'name',name,'kind',kind,'owner',owner,'admin',admin) FROM people ORDER BY name",&[]).await,
        "agent.list" =>rows(db,"SELECT to_jsonb(a) FROM agents a ORDER BY name",&[]).await,
        "agent.show"=>one(db,"SELECT to_jsonb(a) FROM agents a WHERE id=$1 OR name=$1",&[field(d,"agent")?]).await,
        "agent.create"=>{
            personal(a)?;let aid=id("a");
            execute(db,"INSERT INTO agents(id,name,instructions,model,creator) VALUES($1,$2,$3,$4,$5)",&[&aid,slug(field(d,"name")?)?,opt(d,"instructions"),field(d,"model")?,&a.uid]).await?;Ok(json!({"id":aid}))
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
        "member.add"|"member.remove"=>{
            personal(a)?;let mid=mission(app,a,field(d,"mission")?,true).await?;
            let pid=field(d,"principal")?;
            if op=="member.add"{
                let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT id FROM people WHERE id=$1 UNION ALL SELECT id FROM groups WHERE id=$1 UNION ALL SELECT id FROM agents WHERE id=$1)").bind(pid).fetch_one(db).await?;
                if !exists{bail!("Unknown principal");}
                execute(db,"INSERT INTO members VALUES($1,$2) ON CONFLICT DO NOTHING",&[&mid,pid]).await?;
            }else{execute(db,"DELETE FROM members WHERE mission=$1 AND principal=$2",&[&mid,pid]).await?;}
            Ok(json!({"id":pid}))
        }
        "status.list" | "status.create" | "status.update" | "status.remove" => status(db, a, op, d).await,
        "account.role" => {
            owner_access(a)?;
            let person=field(d,"person")?;
            field(d,"role")?;
            let changed=sqlx::query("UPDATE people SET admin=$2 WHERE id=$1 AND NOT owner AND kind='user'").bind(person).bind(role(d)?).execute(db).await?;
            if changed.rows_affected()!=1 {bail!("Choose a regular user account");}
            Ok(json!({"id":person}))
        }
        "group.list"=>rows(db,"SELECT to_jsonb(g)||jsonb_build_object('members',ARRAY(SELECT person FROM group_members WHERE group_id=g.id)) FROM groups g ORDER BY name",&[]).await,
        "group.create"=>{
            admin(a)?;let gid=id("g");execute(db,"INSERT INTO groups VALUES($1,$2)",&[&gid,field(d,"name")?]).await?;Ok(json!({"id":gid}))
        }
        "group.add"|"group.remove"=>{
            admin(a)?;let sql=if op=="group.add"{"INSERT INTO group_members VALUES($1,$2) ON CONFLICT DO NOTHING"}else{"DELETE FROM group_members WHERE group_id=$1 AND person=$2"};
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
