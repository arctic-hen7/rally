mod auth;
mod catalog;
mod chat;
mod engine;
mod git;
mod notify;
mod routines;
mod runtime;
mod search;
mod skills;

use anyhow::{Result, bail};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rally_teams::{field, id, opt};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, broadcast, watch};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct App {
    db: PgPool,
    http: reqwest::Client,
    root: PathBuf,
    base: String,
    events: broadcast::Sender<String>,
    shutdown: watch::Receiver<bool>,
    git_lock: Arc<Mutex<()>>,
    model: Option<model2vec_rs::model::StaticModel>,
}

#[derive(Clone)]
pub struct Actor {
    uid: String,
    name: String,
    owner: bool,
    admin: bool,
    run: Option<String>,
    routine: Option<String>,
}

pub struct Error(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let msg = self.0.to_string();
        let status = match msg.as_str() {
            "Sign in required" => StatusCode::UNAUTHORIZED,
            "Access denied" => StatusCode::FORBIDDEN,
            "Not found" => StatusCode::NOT_FOUND,
            _ if self.0.downcast_ref::<sqlx::Error>().is_some() => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        let message = if status == StatusCode::CONFLICT {
            eprintln!("Database request: {}", self.0);
            "The operation conflicts with existing data".to_string()
        } else {
            msg
        };
        (status, Json(json!({"error":message}))).into_response()
    }
}

pub type ApiResult = std::result::Result<Response, Error>;

pub fn var(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

async fn rows(db: &PgPool, sql: &str, args: &[&str]) -> Result<Value> {
    let mut q = sqlx::query_scalar::<_, Value>(sql);
    for arg in args {
        q = q.bind(*arg);
    }
    Ok(Value::Array(q.fetch_all(db).await?))
}

async fn one(db: &PgPool, sql: &str, args: &[&str]) -> Result<Value> {
    let mut q = sqlx::query_scalar::<_, Value>(sql);
    for arg in args {
        q = q.bind(*arg);
    }
    q.fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Not found"))
}

async fn execute(db: &PgPool, sql: &str, args: &[&str]) -> Result<()> {
    let mut q = sqlx::query(sql);
    for arg in args {
        q = q.bind(*arg);
    }
    q.execute(db).await?;
    Ok(())
}

fn admin(a: &Actor) -> Result<()> {
    if !(a.owner || a.admin) || a.run.is_some() || a.routine.is_some() {
        bail!("Access denied");
    }
    Ok(())
}

fn personal(a: &Actor) -> Result<()> {
    if a.run.is_some() {
        bail!("Access denied");
    }
    Ok(())
}

async fn mission(app: &App, a: &Actor, key: &str, write: bool) -> Result<String> {
    let mid = key.trim_start_matches('#').split(':').next().unwrap_or("");
    let ok: bool = if let Some(run) = &a.run {
        if write {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread JOIN members m ON m.mission=t.mission AND m.principal=s.agent WHERE r.id=$1 AND t.mission=$2 AND r.status='working')").bind(run).bind(mid).fetch_one(&app.db).await?
        } else {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r WHERE r.id=$1 AND r.status='working' AND (NOT EXISTS(SELECT 1 FROM unnest(r.readers) u WHERE NOT can_read(u,$2)) OR r.grants ? $2))").bind(run).bind(mid).fetch_one(&app.db).await?
        }
    } else {
        sqlx::query_scalar("SELECT can_read($1,$2)")
            .bind(&a.uid)
            .bind(mid)
            .fetch_one(&app.db)
            .await?
    };
    if !ok {
        bail!("Access denied");
    }
    Ok(mid.to_string())
}

async fn thread(app: &App, a: &Actor, data: &Value, write: bool) -> Result<Value> {
    let reference = data["thread"]
        .as_str()
        .or(data["mission"].as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing thread or mission"))?;
    let raw = reference.trim_start_matches('#');
    let row = if raw.starts_with("th_") {
        one(
            &app.db,
            "SELECT to_jsonb(t) FROM threads t WHERE id=$1",
            &[raw],
        )
        .await?
    } else if let Some((mid, title)) = raw.split_once(':') {
        one(&app.db,"SELECT to_jsonb(t) FROM threads t WHERE mission=$1 AND lower(title)=lower($2) ORDER BY created LIMIT 1",&[mid,title]).await?
    } else {
        one(
            &app.db,
            "SELECT to_jsonb(t) FROM threads t WHERE mission=$1 AND kind='main'",
            &[raw],
        )
        .await?
    };
    if let Err(e) = mission(app, a, field(&row, "mission")?, write).await {
        let allowed = if let Some(run) = &a.run {
            !write && sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM runs WHERE id=$1 AND status='working' AND grants ? $2)").bind(run).bind(field(&row,"id")?).fetch_one(&app.db).await?
        } else {
            false
        };
        if !allowed {
            return Err(e);
        }
    }
    Ok(row)
}

async fn rpc(
    State(app): State<App>,
    Path(op): Path<String>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> ApiResult {
    let a = auth::actor(&app, &headers).await?;
    let result = match op.as_str() {
        "whoami" => {
            json!({"id":a.uid,"name":a.name,"owner":a.owner,"admin":a.admin,"run":a.run,"routine":a.routine})
        }
        "topic.list" | "topic.create" | "mission.list" | "mission.show" | "mission.create"
        | "mission.update" | "mission.fork" | "group.list" | "group.create" | "group.add"
        | "group.remove" | "member.list" | "member.add" | "member.remove" | "people.list"
        | "agent.list" | "agent.show" | "agent.create" | "agent.update" | "account.service"
        | "account.invite" | "token.create" | "token.list" | "token.revoke" => {
            catalog::call(&app, &a, &op, &data).await?
        }
        s if s.starts_with("status.") || s == "account.role" => {
            catalog::call(&app, &a, s, &data).await?
        }
        s if s.starts_with("skill.") => skills::call(&app, &a, s, &data).await?,
        s if s.starts_with("repo.") => git::call(&app, &a, s, &data).await?,
        s if s.starts_with("routine.") => routines::call(&app, &a, s, &data).await?,
        s if s.starts_with("inbox.") || s.starts_with("device.") => {
            notify::call(&app, &a, s, &data).await?
        }
        "search" => search::query(&app, &a, &data).await?,
        _ => chat::call(&app, &a, &op, &data).await?,
    };
    Ok(Json(result).into_response())
}

async fn live(State(app): State<App>, headers: HeaderMap) -> ApiResult {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::{StreamExt, wrappers::BroadcastStream};
    let a = auth::actor(&app, &headers).await?;
    let mut stop = app.shutdown.clone();
    let stream = BroadcastStream::new(app.events.subscribe())
        .then(move |evt| {
            let app = app.clone();
            let a = a.clone();
            async move {
                let mid = evt.ok()?;
                mission(&app, &a, &mid, false).await.ok()?;
                Some(Ok::<_, std::convert::Infallible>(
                    Event::default().data(mid),
                ))
            }
        })
        .filter_map(|x| x);
    let stream = futures_util::StreamExt::take_until(stream, async move {
        let _ = stop.changed().await;
    });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

pub async fn serve() -> Result<()> {
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(24)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    let mut coordination = db.acquire().await?;
    if !sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock(712737)")
        .fetch_one(&mut *coordination)
        .await?
    {
        bail!("Another Rally server is already running against this database");
    }
    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(712734)")
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let root = PathBuf::from(var("RALLY_DATA", "./data"));
    tokio::fs::create_dir_all(&root).await?;
    let (shutdown_tx, shutdown) = watch::channel(false);
    let model = std::env::var("RALLY_EMBEDDING_DIR")
        .ok()
        .map(|dir| model2vec_rs::model::StaticModel::from_pretrained(dir, None, Some(true), None))
        .transpose()?;
    let app = App {
        db,
        http: reqwest::Client::new(),
        root: root.canonicalize()?,
        base: var("RALLY_URL", "http://localhost:8787")
            .trim_end_matches('/')
            .to_string(),
        events: broadcast::channel(512).0,
        shutdown,
        git_lock: Default::default(),
        model,
    };
    let assets = PathBuf::from(var("RALLY_WEB_DIR", "web/dist"));
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/auth/{op}", post(auth::endpoint))
        .route("/auth/oidc/start", get(auth::oidc_start))
        .route("/auth/oidc/callback", get(auth::oidc_callback))
        .route("/api/{op}", post(rpc))
        .route("/events", get(live))
        .route("/hooks/{id}/{secret}", post(routines::webhook))
        .fallback_service(
            ServeDir::new(&assets).fallback(ServeFile::new(assets.join("index.html"))),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(app.clone());
    let address = var("RALLY_BIND", "127.0.0.1:8787");
    let listener = tokio::net::TcpListener::bind(&address).await?;
    engine::build(&app).await?;
    let worker = tokio::spawn(runtime::supervise(app.clone()));
    let maintenance = tokio::spawn(routines::maintenance(app.clone()));
    println!("Rally listening on {address}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}};
            let _ = shutdown_tx.send(true);
        })
        .await?;
    worker.await??;
    maintenance.await??;
    Ok(())
}
