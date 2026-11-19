use super::*;
use anyhow::Context;
use clap::{Parser, Subcommand};
use std::{
    collections::HashMap,
    fs,
    io::{IsTerminal, Write},
    path::Path,
    sync::Mutex as StdMutex,
};
use tempfile::{NamedTempFile, TempDir};

#[derive(Parser)]
#[command(name = "rally-server", version)]
struct Options {
    /// Operator-owned Codex TOML configuration.
    #[arg(long, env = "RALLY_CODEX_CONFIG")]
    codex_config: PathBuf,
    /// Use the given credential file as the installation identity.
    #[arg(long, env = "RALLY_CODEX_AUTH")]
    auth: Option<PathBuf>,
    /// Let the configured provider supply its own authentication.
    #[arg(long, env = "RALLY_BYPASS_AUTH")]
    bypass_auth: bool,
    #[command(subcommand)]
    action: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    Login {
        #[command(subcommand)]
        action: Option<LoginAction>,
    },
}
#[derive(Subcommand)]
enum LoginAction {
    Status,
}

pub struct Provider {
    pub config: String,
    source: Option<PathBuf>,
    env: Vec<(String, String)>,
    refresh: Mutex<()>,
    leases: StdMutex<HashMap<String, String>>,
    error: StdMutex<Option<String>>,
}
pub struct Lease {
    provider: Arc<Provider>,
    ticket: String,
    pub env: Vec<(String, String)>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.provider.leases.lock().unwrap().remove(&self.ticket);
    }
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing file parent")?;
    fs::create_dir_all(parent)?;
    let mut file = NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

fn read_auth(path: &Path) -> Result<Value> {
    let bytes =
        fs::read(path).with_context(|| format!("Read Codex credentials at {}", path.display()))?;
    let auth: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Invalid Codex credential file {}", path.display()))?;
    if opt(&auth, "OPENAI_API_KEY").is_empty()
        && opt(&auth["tokens"], "access_token").is_empty()
        && opt(&auth, "personal_access_token").is_empty()
    {
        bail!("No Codex credentials found in {}", path.display());
    }
    Ok(auth)
}

async fn login(root: &Path, config: &str, dest: &Path) -> Result<()> {
    let stage = TempDir::new_in(root)?;
    write_private(&stage.path().join("config.toml"), config.as_bytes())?;
    let mut cmd = if engine::local() {
        let mut cmd = tokio::process::Command::new(var("RALLY_CODEX", "codex"));
        cmd.env("CODEX_HOME", stage.path());
        cmd
    } else {
        engine::build_image().await?;
        let mut cmd = engine::engine();
        let host =
            PathBuf::from(std::env::var("RALLY_HOST_DATA")?).join(stage.path().strip_prefix(root)?);
        cmd.args([
            "run",
            "--rm",
            "--interactive",
            "--cap-drop=all",
            "--security-opt=no-new-privileges",
            "--env",
            "CODEX_HOME=/login",
            "--volume",
            &format!("{}:/login:rw", host.display()),
            &var("RALLY_RUNTIME_IMAGE", "localhost/rally-runtime:local"),
            "codex",
        ]);
        cmd
    };
    if !cmd
        .args([
            "-c",
            "cli_auth_credentials_store=\"file\"",
            "login",
            "--device-auth",
        ])
        .status()
        .await?
        .success()
    {
        bail!("Codex login did not complete");
    }
    let auth = read_auth(&stage.path().join("auth.json"))?;
    write_private(dest, &serde_json::to_vec(&auth)?)?;
    println!("Codex login saved to {}", dest.display());
    Ok(())
}

pub async fn start() -> Result<()> {
    let options = Options::parse();
    let config = fs::read_to_string(&options.codex_config)
        .with_context(|| format!("Read Codex config {}", options.codex_config.display()))?;
    config
        .parse::<toml::Table>()
        .context("Invalid Codex configuration")?;
    let root = PathBuf::from(var("RALLY_DATA", "./data"));
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let default = root.join("codex/auth.json");
    let explicit = options
        .auth
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty());
    let path = explicit.unwrap_or(&default);
    if let Some(Action::Login { action }) = options.action {
        if action.is_some() {
            if options.bypass_auth {
                println!("Provider-managed authentication");
            } else {
                read_auth(path)?;
                println!("Codex credentials present at {}", path.display());
            }
            return Ok(());
        }
        return login(&root, &config, &default).await;
    }
    let source = if options.bypass_auth {
        None
    } else {
        if explicit.is_none() && !default.exists() {
            if !std::io::stdin().is_terminal() {
                bail!(
                    "Missing Codex login. Run rally-server --codex-config PATH login, supply --auth PATH, or use --bypass-auth"
                );
            }
            login(&root, &config, &default).await?;
        }
        read_auth(path)?;
        Some(path.clone())
    };
    let env = match std::env::var_os("RALLY_CODEX_ENV_FILE").filter(|v| !v.is_empty()) {
        Some(path) => dotenvy::from_path_iter(path)?.collect::<std::result::Result<Vec<_>, _>>()?,
        None => vec![],
    };
    serve(Arc::new(Provider {
        config,
        source,
        env,
        refresh: Default::default(),
        leases: Default::default(),
        error: Default::default(),
    }))
    .await
}

impl Provider {
    pub fn status(&self) -> Value {
        json!({"mode":if self.source.is_some(){"managed"}else{"provider"},"error":self.error.lock().unwrap().clone()})
    }
    pub fn prepare(self: &Arc<Self>, app: &App, home: &Path) -> Result<Lease> {
        fs::create_dir_all(home)?;
        write_private(&home.join("config.toml"), self.config.as_bytes())?;
        let cache = home.join("auth.json");
        let ticket = rally_teams::secret();
        let mut env = self.env.clone();
        if let Some(source) = &self.source {
            let mut auth = read_auth(source)?;
            if let Some(tokens) = auth["tokens"].as_object_mut() {
                let access = tokens
                    .get("access_token")
                    .and_then(Value::as_str)
                    .context("Missing access token")?;
                self.leases
                    .lock()
                    .unwrap()
                    .insert(ticket.clone(), rally_teams::digest(access));
                tokens.insert("refresh_token".into(), json!(ticket));
                env.push((
                    "CODEX_REFRESH_TOKEN_URL_OVERRIDE".into(),
                    format!(
                        "{}/internal/codex/refresh",
                        var("RALLY_WORKER_URL", &app.base)
                    ),
                ));
            }
            write_private(&cache, &serde_json::to_vec(&auth)?)?;
        } else if cache.try_exists()? {
            fs::remove_file(cache)?;
        }
        Ok(Lease {
            provider: self.clone(),
            ticket,
            env,
        })
    }
}

pub async fn refresh(State(app): State<App>, Json(input): Json<Value>) -> ApiResult {
    let provider = &app.provider;
    let ticket = field(&input, "refresh_token")?;
    let _guard = provider.refresh.lock().await;
    let old = provider
        .leases
        .lock()
        .unwrap()
        .get(ticket)
        .cloned()
        .context("Access denied")?;
    let source = provider.source.as_ref().context("Access denied")?;
    let mut auth = read_auth(source)?;
    let tokens = &auth["tokens"];
    if old == rally_teams::digest(field(tokens, "access_token")?) {
        let snapshot = auth.clone();
        let response=app.http.post("https://auth.openai.com/oauth/token").json(&json!({"grant_type":"refresh_token","client_id":field(&input,"client_id")?,"refresh_token":field(tokens,"refresh_token")?})).send().await?;
        if !response.status().is_success() {
            let message = if response.status().is_client_error() {
                format!(
                    "Codex login requires attention at {}. Reauthenticate the installation.",
                    source.display()
                )
            } else {
                "Codex authentication service is unavailable".into()
            };
            *provider.error.lock().unwrap() = Some(message.clone());
            return Err(anyhow::anyhow!(message).into());
        }
        let update: Value = response.json().await?;
        field(&update, "access_token")?;
        for key in ["access_token", "id_token", "refresh_token"] {
            if let Some(v) = update[key].as_str() {
                auth["tokens"][key] = json!(v);
            }
        }
        auth["last_refresh"] = json!(chrono::Utc::now());
        let latest = read_auth(source)?;
        if latest == snapshot {
            write_private(source, &serde_json::to_vec(&auth)?)?;
        } else {
            auth = latest;
        }
    }
    let mut reply = auth["tokens"].clone();
    reply["refresh_token"] = json!(ticket);
    provider.leases.lock().unwrap().insert(
        ticket.to_string(),
        rally_teams::digest(field(&reply, "access_token")?),
    );
    *provider.error.lock().unwrap() = None;
    Ok(Json(reply).into_response())
}
