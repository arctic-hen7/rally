use super::*;
use super::{chat, engine, git};
use anyhow::Context;
use rally_teams::{digest, secret};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{ChildStdin, ChildStdout},
    task::JoinSet,
};

struct Wire {
    input: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next: i64,
    auth: Option<(App, String, Value)>,
}
impl Wire {
    async fn send(&mut self, data: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&data)?;
        line.push(b'\n');
        self.input.write_all(&line).await?;
        self.input.flush().await?;
        Ok(())
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<i64> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        Ok(id)
    }
    async fn read(&mut self) -> Result<Value> {
        let line = self
            .lines
            .next_line()
            .await?
            .context("Harness disconnected")?;
        Ok(serde_json::from_str(&line)?)
    }
    async fn refresh(&mut self, message: &Value) -> Result<()> {
        let (app, ticket, _) = self.auth.as_ref().context("Unexpected refresh request")?;
        let tokens = provider::refresh_tokens(app, ticket).await?;
        let reply = provider::native_tokens(&tokens)?;
        self.send(json!({"id":message["id"],"result":reply})).await
    }
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.request(method, params).await?;
        tokio::time::timeout(Duration::from_secs(75), async {
            loop {
                let v = self.read().await?;
                if v["method"] == "account/chatgptAuthTokens/refresh" {
                    self.refresh(&v).await?;
                    continue;
                }
                if v["id"] == id && v.get("method").is_none() {
                    if let Some(e) = v.get("error") {
                        bail!("Harness {method}: {e}");
                    }
                    return Ok(v["result"].clone());
                }
            }
        })
        .await?
    }
}

pub async fn summary(app: &App, root: &Path, prompt: &str) -> Result<String> {
    let model: String = sqlx::query_scalar("SELECT wip FROM model_settings")
        .fetch_one(&app.db)
        .await?;
    let home = tempfile::TempDir::new_in(&app.root)?;
    let lease = app.provider.prepare(app, home.path(), false)?;
    let mut env = environment()?;
    env.extend(
        super::secrets::environment(
            app,
            root.file_name()
                .and_then(|s| s.to_str())
                .context("Missing mission directory")?,
            None,
        )
        .await?,
    );
    env.extend(lease.env.clone());
    let args = [
        "exec",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--model",
        &model,
        prompt,
    ];
    let cmd = engine::command(
        app,
        root,
        Some(home.path()),
        &env,
        None,
        &var("RALLY_CODEX", "codex"),
        &args,
    )?;
    engine::output(cmd).await
}

pub fn environment() -> Result<Vec<(String, String)>> {
    let map: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&var("RALLY_RUNTIME_ENV", "{}"))?;
    Ok(map.into_iter().collect())
}

pub async fn supervise(app: App) -> Result<()> {
    execute(
        &app.db,
        "UPDATE runs SET status='interrupted',token_digest=NULL WHERE status='working'",
        &[],
    )
    .await?;
    execute(
        &app.db,
        "UPDATE runs SET status='stopped',token_digest=NULL WHERE status='stopping'",
        &[],
    )
    .await?;
    execute(
        &app.db,
        "UPDATE runs SET status='queued' WHERE status='continuing'",
        &[],
    )
    .await?;
    let mut tasks = JoinSet::new();
    let mut stop = app.shutdown.clone();
    let mut interval = tokio::time::interval(Duration::from_millis(300));
    let parallel = var("RALLY_WORKERS", "4").parse::<usize>()?.max(1);
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            _=interval.tick()=>{
                while tasks.len()<parallel{
                    let token=secret();
                    let row=sqlx::query("WITH picked AS (SELECT id,status FROM runs WHERE status IN ('queued','interrupted') ORDER BY created LIMIT 1 FOR UPDATE SKIP LOCKED) UPDATE runs r SET status='working',token_digest=$1,updated=now() FROM picked p WHERE r.id=p.id RETURNING r.id,p.status").bind(digest(&token)).fetch_optional(&app.db).await?;
                    let Some(row)=row else{break};
                    let app=app.clone();let rid:String=row.get("id");let resume=row.get::<String,_>("status")=="interrupted";
                    tasks.spawn(async move{
                        let outcome=drive(&app,&rid,&token,resume).await;
                        let _ = execute(&app.db, "UPDATE runs SET status='queued' WHERE id=$1 AND status='continuing'", &[&rid]).await;
                        if *app.shutdown.borrow(){let _=execute(&app.db,"UPDATE runs SET status='interrupted',token_digest=NULL WHERE id=$1 AND status='working'",&[&rid]).await;}
                        else if let Err(e)=outcome{eprintln!("Run {rid}: {e:#}");let _=chat::finish(&app,&rid,"failed",&e.to_string()).await;}
                    });
                }
            }
            Some(result)=tasks.join_next(),if !tasks.is_empty()=>{if let Err(e)=result{eprintln!("Worker failed: {e}");}}
        }
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn drive(app: &App, rid: &str, token: &str, resuming: bool) -> Result<()> {
    let r=one(&app.db,"SELECT to_jsonb(r)||jsonb_build_object('native',s.native,'thread',s.thread,'agent',s.agent,'mission',t.mission) FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread WHERE r.id=$1",&[rid]).await?;
    let mid = field(&r, "mission")?;
    let root = git::workspace(app, mid);
    tokio::fs::create_dir_all(&root).await?;
    let repos = git::sync_before_run(app, mid, rid).await?;
    if let Some(broken) = repos
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"]["state"] == "error" && r["status"]["head"].is_null())
    {
        bail!(
            "Repository {} could not be prepared: {}",
            broken["name"],
            broken["status"]["error"]
        );
    }
    let session = field(&r, "session")?;
    let home = app.root.join("sessions").join(session);
    tokio::fs::create_dir_all(&home).await?;
    let context = app.root.join("contexts").join(rid);
    tokio::fs::create_dir_all(&context).await?;
    let bundle = super::skills::bundle(app, mid, field(&r, "agent")?).await?;
    if !context.join("skills").is_dir() {
        rally_teams::materialize_skills(&context.join("skills"), &bundle)?;
    }
    let acp = var("RALLY_ADAPTER", "codex") == "acp";
    let lease = app.provider.prepare(app, &home, !acp)?;
    let mut env = environment()?;
    env.extend(super::secrets::environment(app, mid, Some(field(&r, "agent")?)).await?);
    env.extend(lease.env.clone());
    let git = connections::Lease::new(app, connections::Scope::Mission(mid.into()));
    env.extend(git.env());
    env.extend([
        ("RALLY_URL".into(), var("RALLY_WORKER_URL", &app.base)),
        ("RALLY_TOKEN".into(), token.into()),
        ("RALLY_MISSION".into(), mid.into()),
        ("RALLY_THREAD".into(), field(&r, "thread")?.into()),
        ("RALLY_RUN".into(), rid.into()),
    ]);
    let program = if acp {
        var("RALLY_ACP", "codex-acp")
    } else {
        var("RALLY_CODEX", "codex")
    };
    let args: Vec<&str> = if acp {
        vec![]
    } else {
        vec!["app-server", "--listen", "stdio://"]
    };
    let name = format!("rally-{rid}");
    let mut cmd = engine::command(app, &root, Some(&home), &env, Some(&name), &program, &args)?;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let errors = child.stderr.take().unwrap();
    let error_task = tokio::spawn(async move {
        let mut tail = String::new();
        let mut lines = BufReader::new(errors).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() {
                tail = line.chars().take(640).collect();
            }
        }
        tail
    });
    let mut w = Wire {
        input: child.stdin.take().unwrap(),
        lines: BufReader::new(child.stdout.take().unwrap()).lines(),
        next: 0,
        auth: lease
            .native_login
            .clone()
            .map(|v| (app.clone(), lease.ticket.clone(), v)),
    };
    let mut shutdown = app.shutdown.clone();
    let result = tokio::select! {
        r=conversation(app,&r,resuming,acp,&mut w,&root,&context)=>r.map(Some),
        _=shutdown.changed()=>Ok(None)
    };
    drop(w);
    if !engine::local() {
        engine::stop(&name).await;
    }
    if tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    let tail = error_task.await.unwrap_or_default();
    drop(lease);
    let finished = result.map_err(|e| {
        if tail.is_empty() {
            e
        } else {
            anyhow::anyhow!("{e:#} ({tail})")
        }
    })?;
    if let Some((status, body)) = finished {
        chat::finish(app, rid, status, &body).await?;
    }
    Ok(())
}

async fn conversation(
    app: &App,
    r: &Value,
    resuming: bool,
    acp: bool,
    w: &mut Wire,
    root: &Path,
    context: &Path,
) -> Result<(&'static str, String)> {
    let rid = field(r, "id")?;
    let session = field(r, "session")?;
    let mid = field(r, "mission")?;
    let model = field(r, "model")?;
    let cwd = if engine::local() {
        root.to_string_lossy().to_string()
    } else {
        "/workspace".into()
    };
    let files = if engine::local() {
        context.to_string_lossy().to_string()
    } else {
        "/rally/context".into()
    };
    let instructions = format!(
        "{}\nYou are working in Rally mission #{mid}. Workspace skills, including the Rally CLI guide, are in {files}/skills. Read the relevant SKILL.md. Use rally search and rally read for context. Return your final answer to the parent conversation.",
        opt(r, "instructions")
    );
    let native = r["native"].as_str();
    let native_id;
    if acp {
        let init=w.call("initialize",json!({"protocolVersion":1,"clientCapabilities":{"elicitation":{"form":{}}},"clientInfo":{"name":"rally","version":env!("CARGO_PKG_VERSION")}})).await?;
        let params = json!({"cwd":cwd,"mcpServers":[],"systemPrompt":{"mode":"append","content":[{"type":"text","text":instructions}]}});
        let result = if let Some(native) = native {
            if init["agentCapabilities"]["loadSession"] != true {
                bail!("ACP adapter cannot load a persisted session");
            }
            let mut p = params;
            p["sessionId"] = json!(native);
            w.call("session/load", p).await?
        } else {
            w.call("session/new", params).await?
        };
        native_id = native
            .or(result["sessionId"].as_str())
            .context("ACP omitted the session ID")?
            .to_string();
        if !engine::local()
            && result["modes"]["availableModes"]
                .as_array()
                .is_some_and(|modes| modes.iter().any(|m| m["id"] == "agent-full-access"))
        {
            w.call(
                "session/set_mode",
                json!({"sessionId": native_id, "modeId": "agent-full-access"}),
            )
            .await?;
        }
        if let Some(config) = result["configOptions"]
            .as_array()
            .and_then(|opts| opts.iter().find(|o| o["category"] == "model"))
        {
            if config["currentValue"] != model {
                w.call(
                    "session/set_config_option",
                    json!({"sessionId":native_id,"configId":config["id"],"value":model}),
                )
                .await?;
            }
        } else {
            let models = &result["models"];
            let current = opt(models, "currentModelId");
            if current != model && current.split('[').next() != Some(model) {
                let target = models["availableModels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m["modelId"].as_str())
                    .find(|id| {
                        *id == model || id.strip_prefix(model).is_some_and(|s| s.starts_with('['))
                    })
                    .unwrap_or(model);
                w.call(
                    "session/set_model",
                    json!({"sessionId":native_id,"modelId":target}),
                )
                .await?;
            }
        }
    } else {
        w.call("initialize",json!({"clientInfo":{"name":"rally","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        w.send(json!({"method":"initialized"})).await?;
        if let Some((_, _, login)) = &w.auth {
            w.call("account/login/start", login.clone()).await?;
        }
        let sandbox = if engine::local() {
            "workspace-write"
        } else {
            "danger-full-access"
        };
        let mut params = json!({"cwd":cwd,"model":model,"developerInstructions":instructions,"approvalPolicy":"never","sandbox":sandbox});
        let method = if let Some(native) = native {
            params["threadId"] = json!(native);
            "thread/resume"
        } else {
            "thread/start"
        };
        let start = w.call(method, params).await?;
        native_id = field(&start["thread"], "id")?.to_string();
        if resuming
            && let Some(old) = start["thread"]["turns"].as_array().and_then(|ts| {
                ts.iter()
                    .find(|t| t["id"] == r["native_turn"] && t["status"] == "completed")
            })
        {
            let last = old["items"]
                .as_array()
                .and_then(|items| items.iter().rev().find(|i| i["type"] == "agentMessage"))
                .map(|i| opt(i, "text"))
                .unwrap_or_default();
            return Ok(("done", last.to_owned()));
        }
    }
    execute(
        &app.db,
        "UPDATE sessions SET native=$2 WHERE id=$1",
        &[session, &native_id],
    )
    .await?;
    let trigger = r["next_input"]
        .as_i64()
        .or(r["trigger"].as_i64())
        .context("Missing addressing message")?;
    let addressed = sqlx::query("SELECT author,body,links,native_reply FROM messages WHERE id=$1")
        .bind(trigger)
        .fetch_one(&app.db)
        .await?;
    let unseen=sqlx::query("SELECT m.id,m.author,m.body FROM messages m JOIN sessions s ON s.thread=m.thread WHERE s.id=$1 AND NOT m.silent AND m.id<$2 AND NOT(m.id=ANY(s.delivered)) ORDER BY m.id").bind(session).bind(trigger).fetch_all(&app.db).await?;
    let mut history = String::new();
    let mut delivered = vec![trigger];
    for m in unseen {
        history += &format!(
            "\n### {}\n{}\n",
            m.get::<String, _>("author"),
            m.get::<String, _>("body")
        );
        delivered.push(m.get("id"));
    }
    if history.chars().count() > 100000 {
        tokio::fs::write(context.join("history.md"), history).await?;
        history = format!("Read preceding thread history from {files}/history.md.");
    }
    let was_delivered: bool =
        sqlx::query_scalar("SELECT $2=ANY(delivered) FROM sessions WHERE id=$1")
            .bind(session)
            .bind(trigger)
            .fetch_one(&app.db)
            .await?;
    let prompt = if resuming && was_delivered {
        "We got interrupted, continue".to_string()
    } else {
        format!(
            "{instructions}\n\n{history}\n\n### Addressed by {}\n{}\nReferences: {}",
            addressed.get::<String, _>("author"),
            input_text(&addressed),
            addressed.get::<Value, _>("links")
        )
    };
    let turn;
    let mut prompt_id = 0;
    if acp {
        prompt_id = w
            .request(
                "session/prompt",
                json!({"sessionId":native_id,"prompt":[{"type":"text","text":prompt}]}),
            )
            .await?;
        turn = String::new();
    } else {
        let result = w
            .call(
                "turn/start",
                json!({"threadId":native_id,"input":[{"type":"text","text":prompt}]}),
            )
            .await?;
        turn = field(&result["turn"], "id")?.to_string();
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("UPDATE sessions SET delivered=ARRAY(SELECT DISTINCT unnest(delivered || $2::bigint[])) WHERE id=$1")
        .bind(session).bind(delivered).execute(&mut *tx).await?;
    sqlx::query("UPDATE run_inputs SET sent=true WHERE run=$1 AND message=$2")
        .bind(rid)
        .bind(trigger)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE runs SET native_turn=$2 WHERE id=$1")
        .bind(rid)
        .bind(&turn)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut partial = if resuming {
        opt(r, "partial").to_string()
    } else {
        String::new()
    };
    let mut final_text = String::new();
    let mut tick = tokio::time::interval(Duration::from_millis(180));
    let mut dirty = false;
    let mut steering = std::collections::HashMap::<i64, i64>::new();
    loop {
        tokio::select! {
            msg=w.read()=>{
                let v=msg?;let p=&v["params"];
                if let Some(request)=v["id"].as_i64().and_then(|i|steering.remove(&i)){
                    if let Some(e)=v.get("error"){bail!("Steering failed: {e}");}
                    sqlx::query("UPDATE run_inputs SET sent=true WHERE message=$1 AND run=$2").bind(request).bind(rid).execute(&app.db).await?;
                    sqlx::query("UPDATE sessions SET delivered=array_append(delivered,$2) WHERE id=$1").bind(session).bind(request).execute(&app.db).await?;
                }
                if acp && v["id"]==prompt_id && v.get("method").is_none(){
                    if let Some(e)=v.get("error"){bail!("ACP prompt failed: {e}");}
                    final_text=partial.clone();break;
                }
                match opt(&v,"method"){
                    "account/chatgptAuthTokens/refresh"=>w.refresh(&v).await?,
                    "item/agentMessage/delta"=>{partial.push_str(opt(p,"delta"));dirty=true;}
                    "item/completed" if p["item"]["type"]=="agentMessage"=>{
                        let item=&p["item"];let text=opt(item,"text");
                        if !text.is_empty(){chat::record_work(app,rid,text,opt(item,"id")).await?;final_text=text.into();partial.push('\n');dirty=true;}
                        if item["questions"].is_array(){question(app,rid,Value::Null,item["questions"].clone(),false,opt(item,"id")).await?;}
                    }
                    "session/update" if p["update"]["sessionUpdate"]=="agent_message_chunk"=>{partial.push_str(opt(&p["update"]["content"],"text"));dirty=true;}
                    "turn/completed"=>{if p["turn"]["status"]!="completed"{bail!("Harness turn ended: {}",p["turn"]["error"]);}break;}
                    "item/tool/requestUserInput"|"elicitation/create"=>{
                        question(app,rid,v["id"].clone(),p.clone(),p["isBlocking"].as_bool().unwrap_or(true),"").await?;
                    }
                    "session/request_permission"=>{
                        let allow=p["options"].as_array().and_then(|v|v.iter().find(|o|opt(o,"kind").starts_with("allow"))).and_then(|o|o.get("optionId"));
                        let outcome=allow.map(|id|json!({"outcome":"selected","optionId":id})).unwrap_or(json!({"outcome":"cancelled"}));
                        w.send(json!({"jsonrpc":"2.0","id":v["id"],"result":{"outcome":outcome}})).await?;
                    }
                    _ if v.get("method").is_some() && v.get("id").is_some()=>{w.send(json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Unsupported Rally client request"}})).await?;}
                    _=>{}
                }
            }
            _=tick.tick()=>{
                if dirty{
                    execute(&app.db,"UPDATE runs SET partial=$2,updated=now() WHERE id=$1",&[rid,&partial]).await?;
                    let _=app.events.send(mid.into());dirty=false;
                }
                let state:String=sqlx::query_scalar("SELECT status FROM runs WHERE id=$1").bind(rid).fetch_one(&app.db).await?;
                if state=="stopping"{return Ok(("stopped", String::new()));}
                for input in sqlx::query("SELECT m.id,m.author,m.body,m.native_reply FROM run_inputs i JOIN messages m ON m.id=i.message WHERE i.run=$1 AND NOT i.sent ORDER BY i.id").bind(rid).fetch_all(&app.db).await?{
                    let id:i64=input.get("id");if steering.values().any(|v|*v==id){continue;}
                    let text=format!("{} steers: {}",input.get::<String,_>("author"),input_text(&input));
                    let key=steer(w,acp,&native_id,&turn,&text).await?;
                    steering.insert(key,id);
                }
                for q in sqlx::query("SELECT id,request,answer FROM questions WHERE run=$1 AND answer IS NOT NULL AND NOT sent").bind(rid).fetch_all(&app.db).await?{
                    let request:Value=q.get("request");let answer:Value=q.get("answer");
                    if request.is_null(){steer(w,acp,&native_id,&turn,&format!("Answer to the queued question: {answer}")).await?;}
                    else {let result=if acp{json!({"action":"accept","content":answer})}else{json!({"answers":answer})};
                        w.send(json!({"jsonrpc":"2.0","id":request,"result":result})).await?;}
                    execute(&app.db,"UPDATE questions SET sent=true WHERE id=$1",&[q.get("id")]).await?;
                }
            }
        }
    }
    execute(
        &app.db,
        "UPDATE runs SET partial=$2 WHERE id=$1",
        &[rid, &partial],
    )
    .await?;
    if acp {
        chat::record_work(app, rid, &final_text, &format!("acp:{trigger}")).await?;
    }
    if final_text.is_empty() {
        final_text = partial.trim().to_owned();
    }
    Ok(("done", final_text))
}

fn input_text(message: &sqlx::postgres::PgRow) -> String {
    let body: String = message.get("body");
    match message.get::<Option<Value>, _>("native_reply") {
        Some(replies) => format!(
            "{body}\n<send_user_message_question_reply>{replies}</send_user_message_question_reply>"
        ),
        None => body,
    }
}

async fn steer(w: &mut Wire, acp: bool, native: &str, turn: &str, text: &str) -> Result<i64> {
    if acp {
        w.request("_session/steering",json!({"sessionId":native,"prompt":[{"type":"text","text":text}],"_meta":{"steering":{"idleBehavior":"promptRequired"}}})).await
    } else {
        w.request(
            "turn/steer",
            json!({"threadId":native,"expectedTurnId":turn,"input":[{"type":"text","text":text}]}),
        )
        .await
    }
}
async fn question(
    app: &App,
    rid: &str,
    request: Value,
    prompt: Value,
    blocking: bool,
    key: &str,
) -> Result<()> {
    let mut tx = app.db.begin().await?;
    let qid = format!(
        "{rid}-{}",
        if key.is_empty() {
            request.to_string()
        } else {
            key.to_string()
        }
    );
    let wrote=sqlx::query("INSERT INTO questions(id,run,request,prompt,blocking) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(&qid).bind(rid).bind(request).bind(&prompt).bind(blocking).execute(&mut *tx).await?.rows_affected();
    if wrote > 0 {
        sqlx::query("INSERT INTO messages(thread,author,actor,source,body,origin_key) SELECT r.working_thread,a.name,a.id,'agent',$2,$3 FROM runs r JOIN sessions s ON s.id=r.session JOIN agents a ON a.id=s.agent WHERE r.id=$1 AND r.working_thread IS NOT NULL ON CONFLICT DO NOTHING")
            .bind(rid).bind(chat::question_text(&prompt)).bind(format!("question:{qid}")).execute(&mut *tx).await?;
        let r=sqlx::query("SELECT r.dispatcher,s.thread FROM runs r JOIN sessions s ON s.id=r.session WHERE r.id=$1").bind(rid).fetch_one(&mut *tx).await?;
        chat::notify(
            &mut tx,
            r.get("dispatcher"),
            r.get("thread"),
            None,
            Some(rid),
            "question",
            "Your agent has a question",
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
