use super::*;
use anyhow::Context;
use axum::{body::Body, extract::Request, http::Method};
use std::{collections::HashMap, sync::Mutex as StdMutex, time::Duration};
use url::Url;

pub fn origin(remote: &str) -> Result<String> {
    let url = Url::parse(remote).context("Enter an HTTPS Git URL")?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || remote.contains('\\')
        || remote.chars().any(char::is_control)
    {
        bail!("Use a credential-free HTTPS Git URL");
    }
    Ok(url.origin().ascii_serialization())
}

pub fn client() -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300));
    if let Some(path) = std::env::var_os("RALLY_GIT_CA_FILE").filter(|s| !s.is_empty()) {
        for cert in reqwest::Certificate::from_pem_bundle(&std::fs::read(path)?)? {
            builder = builder.add_root_certificate(cert);
        }
    }
    Ok(builder.build()?)
}

pub async fn validate<'e>(
    db: impl sqlx::PgExecutor<'e>,
    owner: &str,
    remote: &str,
    connection: Option<&str>,
) -> Result<()> {
    let host = origin(remote)?;
    if Url::parse(remote)?.path().trim_matches('/').is_empty() {
        bail!("Enter a repository URL including its path");
    }
    if let Some(cid) = connection {
        let selected = sqlx::query_scalar::<_,String>(
            "SELECT id FROM git_connections WHERE id=$1 AND owner_id=$2 AND origin=$3 FOR KEY SHARE",
        ).bind(cid).bind(owner).bind(host).fetch_optional(db).await?;
        if selected.is_none() {
            bail!("Choose one of your Git connections for this HTTPS host");
        }
    }
    Ok(())
}

#[derive(Clone)]
pub enum Scope {
    Mission(String),
    Run {
        mission: String,
        run: String,
    },
    Job(String),
    Source {
        remote: String,
        owner: String,
        connection: Option<String>,
    },
}
pub type Grants = Arc<StdMutex<HashMap<String, Scope>>>;

pub struct Lease {
    grants: Grants,
    digest: String,
    token: String,
    pub base: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.grants.lock().unwrap().remove(&self.digest);
    }
}
impl Lease {
    pub fn new(app: &App, scope: Scope) -> Self {
        let key = match &scope {
            Scope::Mission(id) | Scope::Job(id) => id.as_str(),
            Scope::Run { mission, .. } => mission.as_str(),
            Scope::Source { .. } => "source",
        };
        let base = format!(
            "{}/git/{key}",
            var("RALLY_WORKER_URL", &app.base).trim_end_matches('/')
        );
        let token = rally_teams::secret();
        let digest = rally_teams::digest(&token);
        app.git_grants.lock().unwrap().insert(digest.clone(), scope);
        Self {
            grants: app.git_grants.clone(),
            digest,
            token,
            base,
        }
    }
    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            ("GIT_CONFIG_COUNT".into(), "2".into()),
            (
                "GIT_CONFIG_KEY_0".into(),
                format!("http.{}/.extraHeader", self.base),
            ),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("Authorization: Bearer {}", self.token),
            ),
            ("GIT_CONFIG_KEY_1".into(), "http.followRedirects".into()),
            ("GIT_CONFIG_VALUE_1".into(), "false".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ]
    }
    pub fn url(&self, name: &str) -> String {
        format!("{}/{name}", self.base)
    }
}

pub fn source(app: &App, remote: &str, owner: &str, connection: Option<&str>) -> Lease {
    Lease::new(
        app,
        Scope::Source {
            remote: remote.into(),
            owner: owner.into(),
            connection: connection.map(str::to_owned),
        },
    )
}

pub async fn probe(
    app: &App,
    owner: &str,
    remote: &str,
    connection: Option<&str>,
    reference: &str,
) -> Result<()> {
    validate(&app.db, owner, remote, connection).await?;
    let lease = source(app, remote, owner, connection);
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(&app.root).envs(lease.env()).args([
        "ls-remote",
        "--exit-code",
        "--",
        &lease.url("repo"),
        reference,
    ]);
    engine::output(cmd).await.map_err(|_| {
        anyhow::anyhow!("Could not read this repository and ref using the selected Git access")
    })?;
    Ok(())
}

pub async fn proxy(
    State(app): State<App>,
    Path((key, name, operation)): Path<(String, String, String)>,
    req: Request,
) -> ApiResult {
    Ok(forward(&app, &key, &name, &operation, req).await?)
}
async fn forward(
    app: &App,
    key: &str,
    name: &str,
    operation: &str,
    req: Request,
) -> Result<Response> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let scope = app
        .git_grants
        .lock()
        .unwrap()
        .get(&rally_teams::digest(token))
        .cloned()
        .context("Access denied")?;
    let query = req.uri().query();
    let service = match (req.method(), operation, query) {
        (&Method::GET, "info/refs", Some("service=git-upload-pack"))
        | (&Method::POST, "git-upload-pack", None) => "git-upload-pack",
        (&Method::GET, "info/refs", Some("service=git-receive-pack"))
        | (&Method::POST, "git-receive-pack", None) => "git-receive-pack",
        _ => bail!("Invalid Git operation"),
    };
    let scope = if let Scope::Run { mission, run } = scope {
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN sessions s ON s.id=r.session JOIN threads t ON t.id=s.thread JOIN members m ON m.mission=t.mission AND m.principal=s.agent WHERE r.id=$1 AND t.mission=$2 AND r.status='working')").bind(run).bind(&mission).fetch_one(&app.db).await?;
        if !active {
            bail!("Access denied");
        }
        Scope::Mission(mission)
    } else {
        scope
    };
    let (remote, owner, connection) = match scope {
        Scope::Mission(mid) if key == mid => {
            let row=sqlx::query("SELECT remote,contributor,connection FROM repos WHERE mission=$1 AND name=$2 AND NOT needs_transfer AND can_read(contributor,mission)")
                .bind(mid).bind(&name).fetch_optional(&app.db).await?.context("Access denied")?;
            (
                row.get::<String, _>("remote"),
                row.get::<String, _>("contributor"),
                row.get::<Option<String>, _>("connection"),
            )
        }
        Scope::Job(id) if key == id => {
            let row=sqlx::query("SELECT j.owner_id,repo FROM jobs j JOIN routines r ON r.id=j.routine AND r.owner_id=j.owner_id CROSS JOIN LATERAL jsonb_array_elements(j.definition->'repos') repo WHERE j.id=$1 AND j.status='working' AND repo->>'name'=$2 AND r.repos @> jsonb_build_array(repo) AND NOT coalesce((repo->>'needs_connection')::boolean,false)")
                .bind(id).bind(&name).fetch_optional(&app.db).await?.context("Access denied")?;
            let repo: Value = row.get("repo");
            (
                field(&repo, "remote")?.into(),
                row.get("owner_id"),
                repo["connection"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned),
            )
        }
        Scope::Source {
            remote,
            owner,
            connection,
        } if key == "source" && name == "repo" && service == "git-upload-pack" => {
            (remote, owner, connection)
        }
        _ => bail!("Access denied"),
    };
    let host = origin(&remote)?;
    if connection.is_none() && service == "git-receive-pack" {
        bail!("Anonymous Git access is read-only");
    }
    let mut url = Url::parse(&remote)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid Git URL"))?
        .pop_if_empty()
        .extend(operation.split('/'));
    url.set_query(query);
    let mut request = app.git_http.request(req.method().clone(), url);
    if let Some(id) = connection {
        let row=sqlx::query("SELECT username,ciphertext FROM git_connections WHERE id=$1 AND owner_id=$2 AND origin=$3")
            .bind(&id).bind(owner).bind(host).fetch_optional(&app.db).await?.context("Access denied")?;
        let password = secrets::decrypt(
            app,
            &id,
            "git-token",
            &mut row.get::<Vec<u8>, _>("ciphertext"),
        )?;
        request = request.basic_auth(row.get::<String, _>("username"), Some(password));
    }
    for header in [
        "git-protocol",
        "content-type",
        "content-encoding",
        "content-length",
    ] {
        if let Some(v) = req.headers().get(header) {
            request = request.header(header, v);
        }
    }
    if req.method() == Method::POST {
        request = request.body(reqwest::Body::wrap_stream(
            req.into_body().into_data_stream(),
        ));
    }
    let result = request.send().await.context("Git upstream unavailable")?;
    if !result.status().is_success() {
        let status = if result.status().as_u16() == 401 {
            axum::http::StatusCode::FORBIDDEN
        } else {
            result.status()
        };
        return Ok((status, "Git upstream declined the request").into_response());
    }
    let kind = if operation == "info/refs" {
        "advertisement"
    } else {
        "result"
    };
    let content = format!("application/x-{service}-{kind}");
    if result
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        != Some(&content)
    {
        bail!("Upstream did not return a Git smart HTTP response");
    }
    Ok(Response::builder()
        .header("content-type", content)
        .header("cache-control", "no-store")
        .body(Body::from_stream(result.bytes_stream()))?)
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    if a.run.is_some() || a.routine.is_some() {
        bail!("Access denied");
    }
    if op == "connection.list" {
        return rows(&app.db,"SELECT jsonb_build_object('id',id,'name',name,'origin',origin,'username',username) FROM git_connections WHERE owner_id=$1 ORDER BY name",&[&a.uid]).await;
    }
    if op == "connection.create" {
        let host = field(d, "host")?;
        let host = if host.contains("://") {
            host.into()
        } else {
            format!("https://{host}")
        };
        let canonical = origin(&host)?;
        if Url::parse(&host)?.path() != "/" {
            bail!("Enter the forge host without a repository path");
        }
        let cid = id("gc");
        let encrypted = secrets::encrypt(app, &cid, "git-token", field(d, "token")?)?;
        sqlx::query("INSERT INTO git_connections VALUES($1,$2,$3,$4,$5,$6)")
            .bind(&cid)
            .bind(&a.uid)
            .bind(field(d, "name")?)
            .bind(canonical)
            .bind(field(d, "username")?)
            .bind(encrypted)
            .execute(&app.db)
            .await?;
        return Ok(json!({"id":cid}));
    }
    let cid = field(d, "connection")?;
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT id FROM git_connections WHERE id=$1 AND owner_id=$2 FOR UPDATE")
        .bind(cid)
        .bind(&a.uid)
        .fetch_optional(&mut *tx)
        .await?
        .context("Access denied")?;
    match op {
        "connection.usage" => {
            let uses:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('type','mission','target',mission,'name',name) FROM repos WHERE connection=$1 UNION ALL SELECT jsonb_build_object('type','skill','target',id,'name',name) FROM skills WHERE connection=$1 UNION ALL SELECT jsonb_build_object('type','routine','target',id,'name',name) FROM routines WHERE repos @> jsonb_build_array(jsonb_build_object('connection',$1::text))").bind(cid).fetch_all(&mut *tx).await?;
            return Ok(json!(uses));
        }
        "connection.update" => {
            let encrypted = secrets::encrypt(app, cid, "git-token", field(d, "token")?)?;
            sqlx::query("UPDATE git_connections SET ciphertext=$2 WHERE id=$1")
                .bind(cid)
                .bind(encrypted)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE skills SET git_paused=false WHERE connection=$1")
                .bind(cid)
                .execute(&mut *tx)
                .await?;
        }
        "connection.remove" => {
            if d["confirm"] != true {
                bail!("Confirm disconnection of all uses of this Git connection");
            }
            sqlx::query("UPDATE repos SET needs_transfer=true,connection=NULL WHERE connection=$1")
                .bind(cid)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE skills SET git_paused=true,connection=NULL,sync_error='Sync paused: Git connection removed' WHERE connection=$1").bind(cid).execute(&mut *tx).await?;
            sqlx::query("UPDATE routines SET enabled=false,repos=(SELECT jsonb_agg(CASE WHEN item->>'connection'=$1 THEN (item-'connection') || jsonb_build_object('needs_connection',true) ELSE item END) FROM jsonb_array_elements(repos) item) WHERE repos @> jsonb_build_array(jsonb_build_object('connection',$1::text))").bind(cid).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM git_connections WHERE id=$1")
                .bind(cid)
                .execute(&mut *tx)
                .await?;
        }
        _ => bail!("Unknown connection operation"),
    }
    tx.commit().await?;
    Ok(json!({"id":cid}))
}

pub async fn selected(
    app: &App,
    actor: &Actor,
    remote: &str,
    requested: Option<&str>,
) -> Result<Option<String>> {
    if let Some(job) = &actor.job {
        let choices: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT DISTINCT nullif(repo->>'connection','') FROM jobs j
             JOIN routines r ON j.routine=r.id AND j.owner_id=r.owner_id
             CROSS JOIN LATERAL jsonb_array_elements(j.definition->'repos') repo
             WHERE j.id=$1 AND j.status='working' AND r.owner_id=$2 AND repo->>'remote'=$3
             AND r.repos @> jsonb_build_array(repo) AND NOT coalesce((repo->>'needs_connection')::boolean,false)"
        ).bind(job).bind(&actor.uid).bind(remote).fetch_all(&app.db).await?;
        if let [only] = choices.as_slice()
            && requested.is_none()
        {
            return Ok(only.clone());
        }
        if requested.is_none() || !choices.iter().any(|c| c.as_deref() == requested) {
            bail!("Choose a repository and connection declared in this routine workspace");
        }
    }
    Ok(requested.map(str::to_owned))
}
