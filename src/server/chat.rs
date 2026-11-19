use super::*;
use sqlx::{Postgres, Transaction};

fn strings(v: &Value) -> impl Iterator<Item = &str> {
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
    if t["kind"] == "working" {
        let rid: String = sqlx::query_scalar("SELECT id FROM runs WHERE working_thread=$1")
            .bind(tid)
            .fetch_one(&app.db)
            .await?;
        return steer_side(app, a, &rid, d).await;
    }
    let mut tx = app.db.begin().await?;
    let result = post_hosted(app, a, d, &t, &mut tx).await?;
    tx.commit().await?;
    let _ = app.events.send(mid.into());
    Ok(result)
}

async fn post_hosted(
    app: &App,
    a: &Actor,
    d: &Value,
    t: &Value,
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Value> {
    let tid = field(t, "id")?;
    let mid = field(t, "mission")?;
    let body = field(d, "body")?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(tid)
        .execute(&mut **tx)
        .await?;
    let actor = if let Some(r) = &a.run {
        sqlx::query_scalar::<_,String>("SELECT agent FROM sessions s JOIN runs r ON r.session=s.id WHERE r.id=$1 AND s.thread=$2").bind(r).bind(tid).fetch_optional(&mut **tx).await?.ok_or_else(||anyhow::anyhow!("Access denied"))?
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
    let Some(msg) = message(tx, tid, &a.name, &actor, source, body, key.as_deref()).await? else {
        let old: i64 =
            sqlx::query_scalar("SELECT id FROM messages WHERE thread=$1 AND origin_key=$2")
                .bind(tid)
                .bind(key)
                .fetch_one(&mut **tx)
                .await?;
        return Ok(json!({"id":old,"duplicate":true}));
    };
    let silent = d["silent"] == true;
    if silent {
        sqlx::query("UPDATE messages SET silent=true WHERE id=$1")
            .bind(msg)
            .execute(&mut **tx)
            .await?;
    }
    if !silent || a.routine.is_none() {
        sqlx::query("INSERT INTO participants VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(tid)
            .bind(&actor)
            .execute(&mut **tx)
            .await?;
    }
    let mut agents = vec![];
    for name in strings(&d["mentions"]) {
        let handle = name.trim_start_matches('@');
        let principal=sqlx::query("SELECT id,true AS agent FROM agents WHERE id=$1 OR name=$1 UNION ALL SELECT id,false FROM people WHERE id=$1 OR email=$1 OR name=$1 LIMIT 1").bind(handle).fetch_optional(&mut **tx).await?.ok_or_else(||anyhow::anyhow!("Unknown mention {handle}"))?;
        let pid: String = principal.get("id");
        let is_agent: bool = principal.get("agent");
        let admitted:bool=sqlx::query_scalar("SELECT can_read($1,$2) OR EXISTS(SELECT 1 FROM members WHERE mission=$2 AND principal=$1)").bind(&pid).bind(mid).fetch_one(&mut **tx).await?;
        if !admitted {
            if is_agent
                && !sqlx::query_scalar::<_, bool>("SELECT can_introduce($1,$2)")
                    .bind(&a.uid)
                    .bind(&pid)
                    .fetch_one(&mut **tx)
                    .await?
            {
                bail!("Access denied");
            }
            if a.run.is_some() || d["confirm"] != true {
                bail!("Confirm that {handle} may read this mission and its entire history");
            }
            sqlx::query("INSERT INTO members VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(mid)
                .bind(&pid)
                .execute(&mut **tx)
                .await?;
        }
        sqlx::query("INSERT INTO participants VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(tid)
            .bind(&pid)
            .execute(&mut **tx)
            .await?;
        if is_agent {
            if !agents.contains(&pid) {
                agents.push(pid);
            }
        } else if pid != a.uid && !silent {
            notify(
                tx,
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
            sqlx::query_scalar("SELECT principal FROM participants p JOIN threads t ON t.id=p.thread WHERE thread=$1 AND (left(p.principal,2) <> 'a_' OR EXISTS(SELECT 1 FROM members WHERE members.mission=t.mission AND members.principal=p.principal))")
                .bind(tid)
                .fetch_all(&mut **tx)
                .await?;
        if members.len() == 2
            && members.contains(&a.uid)
            && let Some(agent) = members.iter().find(|s| s.starts_with("a_"))
        {
            agents.push(agent.clone());
        }
    }
    let (readers, mut grants) = if let Some(parent) = &a.run {
        let r = sqlx::query("SELECT readers,grants FROM runs WHERE id=$1 AND status='working'")
            .bind(parent)
            .fetch_one(&mut **tx)
            .await?;
        (
            r.get::<Vec<String>, _>("readers"),
            r.get::<Value, _>("grants"),
        )
    } else {
        (vec![a.uid.clone()], json!({}))
    };
    let mut links = Vec::new();
    for reference in strings(&d["refs"]) {
        let refdata = json!({"thread":reference});
        let linked = thread(app, a, &refdata, false).await?;
        let key = if reference.contains(':') || reference.starts_with("th_") {
            field(&linked, "id")?
        } else {
            field(&linked, "mission")?
        };
        grants[key] = json!(a.uid);
        links.push(json!({"label":reference,"target":key}));
    }
    sqlx::query("UPDATE messages SET links=$2 WHERE id=$1")
        .bind(msg)
        .bind(json!(links))
        .execute(&mut **tx)
        .await?;
    let mut runs = vec![];
    for aid in agents {
        let session:String=sqlx::query_scalar("INSERT INTO sessions(id,thread,agent) VALUES($1,$2,$3) ON CONFLICT(thread,agent) DO UPDATE SET agent=excluded.agent RETURNING id").bind(id("ss")).bind(tid).bind(&aid).fetch_one(&mut **tx).await?;
        let active:Option<String>=sqlx::query_scalar("SELECT id FROM runs WHERE session=$1 AND status IN ('queued','working','interrupted','stopping','continuing') FOR UPDATE").bind(&session).fetch_optional(&mut **tx).await?;
        let rid = if let Some(r) = active {
            if implicit {
                bail!("The agent is working. Mention it to steer or open the working pane.");
            }
            sqlx::query("UPDATE runs SET readers=ARRAY(SELECT DISTINCT unnest(readers || $2::text[])),grants=grants || $3 WHERE id=$1").bind(&r).bind(&readers).bind(&grants).execute(&mut **tx).await?;
            sqlx::query("INSERT INTO run_inputs(run,message,kind) VALUES($1,$2,'steer')")
                .bind(&r)
                .bind(msg)
                .execute(&mut **tx)
                .await?;
            r
        } else {
            let r = id("run");
            let work = id("th");
            sqlx::query("INSERT INTO threads(id,mission,title,kind) SELECT $1,$2,name||' · '||$3,'working' FROM agents WHERE id=$4").bind(&work).bind(mid).bind(msg.to_string()).bind(&aid).execute(&mut **tx).await?;
            sqlx::query("INSERT INTO runs(id,session,dispatcher,trigger,status,readers,grants,model,instructions,working_thread) SELECT $1,$2,$3,$4,'queued',$7::text[],$5,model,instructions,$8 FROM agents WHERE id=$6").bind(&r).bind(&session).bind(&a.uid).bind(msg).bind(&grants).bind(&aid).bind(&readers).bind(&work).execute(&mut **tx).await?;
            r
        };
        runs.push(rid);
    }
    if !silent {
        sqlx::query("INSERT INTO notifications(person,thread,message,reason,summary) SELECT person,$1,$2,'following',$3 FROM subscriptions WHERE thread=$1 AND mode='all' AND person!=$4 AND can_read(person,$5) AND NOT EXISTS(SELECT 1 FROM notifications n WHERE n.message=$2 AND n.person=subscriptions.person)").bind(tid).bind(msg).bind(format!("{} posted in {}",a.name,field(t,"title")?)).bind(&a.uid).bind(mid).execute(&mut **tx).await?;
    }
    Ok(json!({"id":msg,"runs":runs}))
}

pub async fn record_work(app: &App, rid: &str, body: &str, key: &str) -> Result<()> {
    if body.is_empty() {
        return Ok(());
    }
    sqlx::query("INSERT INTO messages(thread,author,actor,source,body,origin_key) SELECT r.working_thread,a.name,a.id,'agent',$2,$3 FROM runs r JOIN sessions s ON s.id=r.session JOIN agents a ON a.id=s.agent WHERE r.id=$1 AND r.working_thread IS NOT NULL ON CONFLICT(thread,origin_key) DO NOTHING").bind(rid).bind(body).bind(key).execute(&app.db).await?;
    Ok(())
}

async fn steer_side(app: &App, a: &Actor, rid: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    let mut tx = app.db.begin().await?;
    let row=sqlx::query("SELECT r.working_thread,t.mission FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread WHERE r.id=$1 AND r.status IN ('queued','working','interrupted','continuing') AND can_read($2,t.mission) FOR UPDATE OF r").bind(rid).bind(&a.uid).fetch_optional(&mut *tx).await?.ok_or_else(||anyhow::anyhow!("Run ended or inaccessible"))?;
    let tid: Option<String> = row.get("working_thread");
    let tid = tid.ok_or_else(|| anyhow::anyhow!("Run has no working conversation"))?;
    let mut grants = json!({});
    for reference in strings(&d["refs"]) {
        let linked = thread(app, a, &json!({"thread":reference}), false).await?;
        let key = if reference.contains(':') || reference.starts_with("th_") {
            field(&linked, "id")?
        } else {
            field(&linked, "mission")?
        };
        grants[key] = json!(a.uid);
    }
    let msg = message(
        &mut tx,
        &tid,
        &a.name,
        &a.uid,
        "rally",
        field(d, "body")?,
        None,
    )
    .await?
    .unwrap();
    if d["silent"] == true {
        sqlx::query("UPDATE messages SET silent=true WHERE id=$1")
            .bind(msg)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE runs SET readers=ARRAY(SELECT DISTINCT unnest(readers || ARRAY[$2::text])),grants=grants||$3 WHERE id=$1").bind(rid).bind(&a.uid).bind(grants).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO run_inputs(run,message,kind) VALUES($1,$2,'steer')")
        .bind(rid)
        .bind(msg)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let _ = app.events.send(row.get("mission"));
    Ok(json!({"id":msg,"run":rid}))
}

pub async fn finish(app: &App, rid: &str, status: &str, body: &str) -> Result<()> {
    let mut tx = app.db.begin().await?;
    let r=sqlx::query("SELECT s.thread,s.agent,t.mission,a.name,r.dispatcher FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread JOIN agents a ON a.id=s.agent WHERE r.id=$1 AND r.status IN ('working','stopping','interrupted') FOR UPDATE OF r").bind(rid).fetch_optional(&mut *tx).await?;
    let Some(r) = r else { return Ok(()) };
    let tid: &str = r.get("thread");
    let name: &str = r.get("name");
    if status == "done" {
        let queued = sqlx::query("UPDATE runs SET status='continuing', next_input=i.message, native_turn=NULL, token_digest=NULL FROM (SELECT message FROM run_inputs WHERE run=$1 AND NOT sent ORDER BY id LIMIT 1) i WHERE runs.id=$1 AND runs.status='working'")
            .bind(rid).execute(&mut *tx).await?;
        if queued.rows_affected() == 1 {
            tx.commit().await?;
            return Ok(());
        }
    }
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
    if let Some(n) = msg {
        sqlx::query("INSERT INTO notifications(person,thread,message,run,reason,summary) SELECT person,$1,$2,$3,'following',$4 FROM subscriptions WHERE thread=$1 AND mode='all' AND can_read(person,$5) AND person!=$6").bind(tid).bind(n).bind(rid).bind(format!("{name} replied")).bind(r.get::<String,_>("mission")).bind(r.get::<String,_>("dispatcher")).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    let _ = app.events.send(r.get::<String, _>("mission"));
    Ok(())
}

fn question_fields(prompt: &Value) -> Vec<(String, String)> {
    if let Some(list) = prompt.as_array().or_else(|| prompt["questions"].as_array()) {
        return list
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let key = item["id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| i.to_string());
                let title = item["question"]
                    .as_str()
                    .or(item["title"].as_str())
                    .unwrap_or(&key);
                (key.clone(), title.to_string())
            })
            .collect();
    }
    let properties = prompt["requestedSchema"]["properties"]
        .as_object()
        .or_else(|| prompt["schema"]["properties"].as_object());
    if let Some(properties) = properties {
        return properties
            .iter()
            .map(|(k, v)| (k.clone(), v["title"].as_str().unwrap_or(k).to_string()))
            .collect();
    }
    vec![(
        "answer".into(),
        prompt["message"].as_str().unwrap_or("Question").into(),
    )]
}

pub fn question_text(prompt: &Value) -> String {
    question_fields(prompt)
        .into_iter()
        .map(|(_, title)| title)
        .collect::<Vec<_>>()
        .join("\n")
}

async fn answer_question(app: &App, a: &Actor, run: &Value, d: &Value) -> Result<()> {
    let rid = field(run, "id")?;
    let qid = field(d, "question")?;
    let mut tx = app.db.begin().await?;
    let q = sqlx::query(
        "SELECT request,prompt FROM questions WHERE id=$1 AND run=$2 AND answer IS NULL FOR UPDATE",
    )
    .bind(qid)
    .bind(rid)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Question already resolved or missing"))?;
    let prompt: Value = q.get("prompt");
    let answer = d["answer"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected answers keyed by question ID"))?;
    let mut replies = Vec::new();
    let mut lines = Vec::new();
    for (key, title) in question_fields(&prompt) {
        let value = answer
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("Missing answer for {title}"))?;
        let text = if let Some(items) = value["answers"].as_array() {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        };
        if text.trim().is_empty() {
            bail!("Empty answer for {title}");
        }
        lines.push(format!("{title} — {text}"));
        replies.push(json!({"questionItemId":qid.strip_prefix(&format!("{rid}-")).unwrap_or(qid),"question":title,"answer":text}));
    }
    let body = lines.join("\n");
    let asynchronous = q.get::<Value, _>("request").is_null();
    if asynchronous {
        let post = json!({"thread":run["thread"],"body":body,"mentions":[run["agent"]],"key":qid});
        let t = thread(app, a, &post, true).await?;
        let message = post_hosted(app, a, &post, &t, &mut tx).await?;
        sqlx::query("UPDATE messages SET native_reply=$2 WHERE id=$1")
            .bind(message["id"].as_i64())
            .bind(json!(replies))
            .execute(&mut *tx)
            .await?;
    } else {
        let changed = sqlx::query("UPDATE runs SET readers=ARRAY(SELECT DISTINCT unnest(readers || ARRAY[$2::text])) WHERE id=$1 AND status='working'").bind(rid).bind(&a.uid).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            bail!("Run ended");
        }
        message(
            &mut tx,
            field(run, "working_thread")?,
            &a.name,
            &a.uid,
            "rally",
            &body,
            Some(qid),
        )
        .await?;
    }
    sqlx::query("UPDATE questions SET answer=$1,answered_by=$2,sent=$3 WHERE id=$4")
        .bind(&d["answer"])
        .bind(&a.uid)
        .bind(asynchronous)
        .bind(qid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let _ = app.events.send(field(run, "mission")?.to_owned());
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
    if let Some(n) = n {
        let shas = d["commits"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, v)| {
                rally_teams::slug(name).is_ok()
                    && v.as_str().is_some_and(|s| {
                        matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
                    })
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        sqlx::query("UPDATE messages SET commits=$2 WHERE id=$1")
            .bind(n)
            .bind(json!(shas))
            .execute(&mut *tx)
            .await?;
    }
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
            let messages=rows(db,"SELECT to_jsonb(m)-'embedding'-'origin_key'-'native_reply' FROM messages m WHERE thread=$1 AND NOT silent ORDER BY id",&[tid]).await?;
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
                execute(db,"UPDATE runs SET status=CASE WHEN status IN ('queued','continuing','interrupted') THEN 'stopped' ELSE 'stopping' END WHERE id=$1 AND status IN ('queued','working','interrupted','continuing')",&[rid]).await?;
            }
            if op == "run.steer" {
                return steer_side(app, a, rid, d).await;
            }
            if op == "run.answer" {
                answer_question(app, a, &r, d).await?;
            }
            r["activity"]=rows(db,"SELECT jsonb_build_object('id',id,'author',author,'body',body,'source',source) FROM messages WHERE thread=$1 AND NOT silent ORDER BY id",&[opt(&r,"working_thread")]).await?;
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
