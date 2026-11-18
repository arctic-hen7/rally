use super::*;
use sqlx::{Postgres, Transaction};

fn strings<'a>(v: &'a Value) -> impl Iterator<Item = &'a str> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str)
}

async fn message(
    tx: &mut Transaction<'_, Postgres>,
    tid: &str,
    author: &str,
    actor: &str,
    source: &str,
    body: &str,
    key: Option<&str>,
) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar("INSERT INTO messages(thread,author,actor,source,body,origin_key,commits) SELECT $1,$2,$3,$4,$5,$6,coalesce((SELECT jsonb_object_agg(name,status->>'head') FROM repos WHERE mission=t.mission AND status->>'head' IS NOT NULL),'{}') FROM threads t WHERE t.id=$1 ON CONFLICT(thread,origin_key) DO NOTHING RETURNING id").bind(tid).bind(author).bind(actor).bind(source).bind(body).bind(key).fetch_optional(&mut **tx).await?)
}

pub async fn notify(
    tx: &mut Transaction<'_, Postgres>,
    person: &str,
    tid: &str,
    msg: Option<i64>,
    run: Option<&str>,
    reason: &str,
    summary: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO notifications(person,thread,message,run,reason,summary) SELECT $1,$2,$3,$4,$5,$6 WHERE EXISTS(SELECT 1 FROM threads WHERE id=$2 AND can_read($1,mission)) AND NOT EXISTS(SELECT 1 FROM notifications WHERE person=$1 AND message=$3)").bind(person).bind(tid).bind(msg).bind(run).bind(reason).bind(summary).execute(&mut **tx).await?;
    Ok(())
}

async fn post(app: &App, a: &Actor, d: &Value) -> Result<Value> {
    let t = thread(app, a, d, true).await?;
    let tid = field(&t, "id")?;
    let mid = field(&t, "mission")?;
    if t["kind"] == "external" {
        bail!("External sessions are captured from their original client");
    }
    let body = field(d, "body")?;
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(tid)
        .execute(&mut *tx)
        .await?;
    let actor = if let Some(r) = &a.run {
        sqlx::query_scalar::<_,String>("SELECT agent FROM sessions s JOIN runs r ON r.session=s.id WHERE r.id=$1 AND s.thread=$2").bind(r).bind(tid).fetch_optional(&mut *tx).await?.ok_or_else(||anyhow::anyhow!("Access denied"))?
    } else {
        a.routine.clone().unwrap_or_else(|| a.uid.clone())
    };
    let source = if a.run.is_some() {
        "agent"
    } else if a.routine.is_some() {
        "routine"
    } else {
        "rally"
    };
    let key = d["key"].as_str().map(|k| format!("{actor}:{k}"));
    let Some(msg) = message(&mut tx, tid, &a.name, &actor, source, body, key.as_deref()).await?
    else {
        let old: i64 =
            sqlx::query_scalar("SELECT id FROM messages WHERE thread=$1 AND origin_key=$2")
                .bind(tid)
                .bind(key)
                .fetch_one(&mut *tx)
                .await?;
        return Ok(json!({"id":old,"duplicate":true}));
    };
    sqlx::query("INSERT INTO participants VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(tid)
        .bind(&actor)
        .execute(&mut *tx)
        .await?;
    let mut agents = vec![];
    for name in strings(&d["mentions"]) {
        let handle = name.trim_start_matches('@');
        let principal=sqlx::query("SELECT id,true AS agent FROM agents WHERE id=$1 OR name=$1 UNION ALL SELECT id,false FROM people WHERE id=$1 OR email=$1 OR name=$1 LIMIT 1").bind(handle).fetch_optional(&mut *tx).await?.ok_or_else(||anyhow::anyhow!("Unknown mention {handle}"))?;
        let pid: String = principal.get("id");
        let is_agent: bool = principal.get("agent");
        let admitted:bool=sqlx::query_scalar("SELECT can_read($1,$2) OR EXISTS(SELECT 1 FROM members WHERE mission=$2 AND principal=$1)").bind(&pid).bind(mid).fetch_one(&mut *tx).await?;
        if !admitted {
            if a.run.is_some() || d["confirm"] != true {
                bail!("Confirm that {handle} may read this mission and its entire history");
            }
            sqlx::query("INSERT INTO members VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(mid)
                .bind(&pid)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO participants VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(tid)
            .bind(&pid)
            .execute(&mut *tx)
            .await?;
        if is_agent {
            if !agents.contains(&pid) {
                agents.push(pid);
            }
        } else if pid != a.uid {
            notify(
                &mut tx,
                &pid,
                tid,
                Some(msg),
                None,
                "mention",
                &format!("{} mentioned you", a.name),
            )
            .await?;
        }
    }
    let implicit = agents.is_empty()
        && strings(&d["mentions"]).next().is_none()
        && a.run.is_none()
        && a.routine.is_none();
    if implicit {
        let members: Vec<String> =
            sqlx::query_scalar("SELECT principal FROM participants WHERE thread=$1")
                .bind(tid)
                .fetch_all(&mut *tx)
                .await?;
        if members.len() == 2 && members.contains(&a.uid) {
            if let Some(agent) = members.iter().find(|s| s.starts_with("a_")) {
                agents.push(agent.clone());
            }
        }
    }
    let mut grants = json!({});
    for reference in strings(&d["refs"]) {
        let refdata = json!({"thread":reference});
        let linked = thread(app, a, &refdata, false).await?;
        let key = if reference.contains(':') || reference.starts_with("th_") {
            field(&linked, "id")?
        } else {
            field(&linked, "mission")?
        };
        grants[key] = json!(a.uid);
    }
    let mut runs = vec![];
    for aid in agents {
        let session:String=sqlx::query_scalar("INSERT INTO sessions(id,thread,agent) VALUES($1,$2,$3) ON CONFLICT(thread,agent) DO UPDATE SET agent=excluded.agent RETURNING id").bind(id("ss")).bind(tid).bind(&aid).fetch_one(&mut *tx).await?;
        let active:Option<String>=sqlx::query_scalar("SELECT id FROM runs WHERE session=$1 AND status IN ('queued','working','interrupted','stopping') FOR UPDATE").bind(&session).fetch_optional(&mut *tx).await?;
        let rid = if let Some(r) = active {
            if implicit {
                bail!("The agent is working. Mention it to steer or open the working pane.");
            }
            sqlx::query("UPDATE runs SET readers=ARRAY(SELECT DISTINCT unnest(readers || ARRAY[$2::text])),grants=grants || $3 WHERE id=$1").bind(&r).bind(&a.uid).bind(&grants).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO run_inputs(run,message,kind) VALUES($1,$2,'steer')")
                .bind(&r)
                .bind(msg)
                .execute(&mut *tx)
                .await?;
            r
        } else {
            let r = id("run");
            sqlx::query("INSERT INTO runs(id,session,dispatcher,trigger,status,readers,grants,model,instructions) SELECT $1,$2,$3,$4,'queued',ARRAY[$3::text],$5,model,instructions FROM agents WHERE id=$6").bind(&r).bind(&session).bind(&a.uid).bind(msg).bind(&grants).bind(&aid).execute(&mut *tx).await?;
            r
        };
        runs.push(rid);
    }
    sqlx::query("INSERT INTO notifications(person,thread,message,reason,summary) SELECT person,$1,$2,'following',$3 FROM subscriptions WHERE thread=$1 AND mode='all' AND person!=$4 AND can_read(person,$5) AND NOT EXISTS(SELECT 1 FROM notifications n WHERE n.message=$2 AND n.person=subscriptions.person)").bind(tid).bind(msg).bind(format!("{} posted in {}",a.name,field(&t,"title")?)).bind(&a.uid).bind(mid).execute(&mut *tx).await?;
    tx.commit().await?;
    let _ = app.events.send(mid.into());
    Ok(json!({"id":msg,"runs":runs}))
}

pub async fn finish(app: &App, rid: &str, status: &str, body: &str) -> Result<()> {
    let mut tx = app.db.begin().await?;
    let r=sqlx::query("SELECT s.thread,s.agent,t.mission,a.name,r.dispatcher FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread JOIN agents a ON a.id=s.agent WHERE r.id=$1 AND r.status IN ('working','stopping','interrupted') FOR UPDATE OF r").bind(rid).fetch_optional(&mut *tx).await?;
    let Some(r) = r else { return Ok(()) };
    let tid: &str = r.get("thread");
    let name: &str = r.get("name");
    let msg = if status == "done" && !body.is_empty() {
        message(&mut tx, tid, name, r.get("agent"), "agent", body, Some(rid)).await?
    } else {
        None
    };
    if let Some(n) = msg {
        sqlx::query("UPDATE sessions SET delivered=array_append(delivered,$2) WHERE id=(SELECT session FROM runs WHERE id=$1)").bind(rid).bind(n).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE runs SET status=$2,error=$3,token_digest=NULL,updated=now() WHERE id=$1")
        .bind(rid)
        .bind(status)
        .bind(if status == "failed" { Some(body) } else { None })
        .execute(&mut *tx)
        .await?;
    notify(
        &mut tx,
        r.get("dispatcher"),
        tid,
        msg,
        Some(rid),
        status,
        &format!("{name} · {status}"),
    )
    .await?;
    tx.commit().await?;
    let _ = app.events.send(r.get::<String, _>("mission"));
    Ok(())
}

async fn capture(app: &App, a: &Actor, d: &Value) -> Result<Value> {
    personal(a)?;
    let mid = mission(app, a, field(d, "mission")?, true).await?;
    let source = field(d, "source")?;
    let ext = format!("{}:{source}:{}", a.uid, field(d, "session")?);
    let mut tx = app.db.begin().await?;
    let tid:String=sqlx::query_scalar("INSERT INTO threads(id,mission,title,kind,external_key) VALUES($1,$2,$3,'external',$4) ON CONFLICT(mission,external_key) DO UPDATE SET external_key=excluded.external_key RETURNING id").bind(id("th")).bind(&mid).bind(d["title"].as_str().unwrap_or(source)).bind(ext).fetch_one(&mut *tx).await?;
    let who = if d["role"] == "user" { &a.name } else { source };
    let n = message(
        &mut tx,
        &tid,
        who,
        &a.uid,
        source,
        field(d, "body")?,
        Some(field(d, "key")?),
    )
    .await?;
    tx.commit().await?;
    let _ = app.events.send(mid);
    Ok(json!({"id":n,"thread":tid}))
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    let db = &app.db;
    match op {
        "post" => post(app, a, d).await,
        "capture" => capture(app, a, d).await,
        "thread.list" => {
            let mid = mission(app, a, field(d, "mission")?, false).await?;
            rows(db,"SELECT to_jsonb(t) FROM threads t WHERE mission=$1 ORDER BY (kind='main') DESC,created DESC",&[&mid]).await
        }
        "thread.create" => {
            personal(a)?;
            let mid = mission(app, a, field(d, "mission")?, true).await?;
            let tid = id("th");
            let mut tx = db.begin().await?;
            sqlx::query("INSERT INTO threads(id,mission,title,kind) VALUES($1,$2,$3,'hosted')")
                .bind(&tid)
                .bind(mid)
                .bind(field(d, "title")?)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO participants VALUES($1,$2)")
                .bind(&tid)
                .bind(&a.uid)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(json!({"id":tid}))
        }
        "thread.rename" | "thread.subscribe" | "thread.seen" => {
            personal(a)?;
            let t = thread(app, a, d, true).await?;
            let tid = field(&t, "id")?;
            match op {
                "thread.rename" => {
                    execute(
                        db,
                        "UPDATE threads SET title=$1 WHERE id=$2",
                        &[field(d, "title")?, tid],
                    )
                    .await?
                }
                "thread.subscribe" => {
                    let mode = field(d, "mode")?;
                    if !["all", "participating", "muted"].contains(&mode) {
                        bail!("Unknown subscription mode");
                    }
                    execute(db,"INSERT INTO subscriptions(person,thread,mode) VALUES($1,$2,$3) ON CONFLICT(person,thread) DO UPDATE SET mode=excluded.mode",&[&a.uid,tid,mode]).await?;
                }
                _ => {
                    execute(db,"INSERT INTO subscriptions(person,thread,mode,seen) VALUES($1,$2,'participating',coalesce((SELECT max(id) FROM messages WHERE thread=$2),0)) ON CONFLICT(person,thread) DO UPDATE SET seen=excluded.seen",&[&a.uid,tid]).await?;
                    execute(
                        db,
                        "UPDATE notifications SET seen=true WHERE person=$1 AND thread=$2",
                        &[&a.uid, tid],
                    )
                    .await?;
                }
            }
            Ok(json!({"id":tid}))
        }
        "read" | "thread.show" => {
            let t = thread(app, a, d, false).await?;
            let tid = field(&t, "id")?;
            let messages=rows(db,"SELECT to_jsonb(m)-'embedding'-'origin_key' FROM messages m WHERE thread=$1 ORDER BY id",&[tid]).await?;
            let people=rows(db,"SELECT jsonb_build_object('id',principal,'name',coalesce(p.name,a.name,r.name)) FROM participants q LEFT JOIN people p ON p.id=principal LEFT JOIN agents a ON a.id=principal LEFT JOIN routines r ON r.id=principal WHERE thread=$1",&[tid]).await?;
            let runs=rows(db,"SELECT to_jsonb(v) FROM (SELECT r.id,r.trigger,r.status,r.updated,s.agent,a.name FROM runs r JOIN sessions s ON s.id=r.session JOIN agents a ON a.id=s.agent WHERE s.thread=$1 ORDER BY r.created DESC) v",&[tid]).await?;
            let mode: Option<String> =
                sqlx::query_scalar("SELECT mode FROM subscriptions WHERE thread=$1 AND person=$2")
                    .bind(tid)
                    .bind(&a.uid)
                    .fetch_optional(db)
                    .await?;
            Ok(
                json!({"thread":t,"messages":messages,"participants":people,"runs":runs,"subscription":mode.unwrap_or_else(||"participating".into())}),
            )
        }
        "run.list" => {
            let mid = mission(app, a, field(d, "mission")?, false).await?;
            rows(db,"SELECT to_jsonb(v) FROM (SELECT r.id,r.status,s.thread,s.agent,a.name,r.trigger,r.created FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread JOIN agents a ON a.id=s.agent WHERE t.mission=$1 ORDER BY r.created DESC) v",&[&mid]).await
        }
        "run.show" | "run.stop" | "run.steer" | "run.answer" => {
            let rid = field(d, "run")?;
            let mut r=one(db,"SELECT (to_jsonb(r)-'token_digest'-'readers'-'grants'-'instructions')||jsonb_build_object('thread',s.thread,'agent',s.agent,'name',a.name,'mission',t.mission) FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread JOIN agents a ON a.id=s.agent WHERE r.id=$1",&[rid]).await?;
            mission(app, a, field(&r, "mission")?, op != "run.show").await?;
            if op != "run.show" {
                personal(a)?;
            }
            if op == "run.stop" {
                execute(db,"UPDATE runs SET status=CASE WHEN status='queued' THEN 'stopped' ELSE 'stopping' END WHERE id=$1 AND status IN ('queued','working','interrupted')",&[rid]).await?;
            }
            if op == "run.steer" {
                if !["working", "queued", "interrupted"].contains(&opt(&r, "status")) {
                    bail!("Run has ended");
                }
                let mut input = d.clone();
                input["thread"] = r["thread"].clone();
                input["mentions"] = json!([r["agent"]]);
                return post(app, a, &input).await;
            }
            if op == "run.answer" {
                let mut tx = db.begin().await?;
                let result=sqlx::query("UPDATE questions SET answer=$1,answered_by=$2 WHERE id=$3 AND run=$4 AND answer IS NULL AND EXISTS(SELECT 1 FROM runs WHERE id=$4 AND status='working')").bind(&d["answer"]).bind(&a.uid).bind(field(d,"question")?).bind(rid).execute(&mut *tx).await?;
                if result.rows_affected() != 1 {
                    bail!("Question already resolved or run ended");
                }
                sqlx::query("UPDATE runs SET readers=ARRAY(SELECT DISTINCT unnest(readers || ARRAY[$2::text])) WHERE id=$1").bind(rid).bind(&a.uid).execute(&mut *tx).await?;
                tx.commit().await?;
            }
            r["questions"] = rows(
                db,
                "SELECT to_jsonb(q)-'request' FROM questions q WHERE run=$1 ORDER BY id",
                &[rid],
            )
            .await?;
            Ok(r)
        }
        _ => bail!("Unknown operation {op}"),
    }
}
