mod git_remote;
mod headers;
mod hooks;
mod oidc;
mod secrets;
mod workspace;

use anyhow::{Context, Result, bail};
use clap::{Arg, ArgAction, Command};
use rally_teams::{config_dir, field, opt, read_json, write_json};
use serde_json::{Value, json};
use std::{
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

fn profile_url(config: &Value) -> String {
    std::env::var("RALLY_URL")
        .unwrap_or_else(|_| {
            config["url"]
                .as_str()
                .unwrap_or("http://localhost:8787")
                .to_owned()
        })
        .trim_end_matches('/')
        .to_owned()
}

pub struct Client {
    pub url: String,
    token: String,
    http: reqwest::blocking::Client,
    headers: std::cell::RefCell<headers::Headers>,
}
impl Client {
    fn new(override_url: Option<&str>) -> Result<Self> {
        let c = read_json(&config_dir().join("config.json"))?;
        let saved = c["url"].as_str().unwrap_or("http://localhost:8787");
        let default_url = profile_url(&c);
        let url = override_url.unwrap_or(&default_url).trim_end_matches('/');
        let target = url::Url::parse(url).context("Invalid Rally URL")?;
        anyhow::ensure!(
            matches!(target.scheme(), "http" | "https")
                && target.username().is_empty()
                && target.password().is_none()
                && target.query().is_none()
                && target.fragment().is_none(),
            "Expected an HTTP(S) Rally URL without credentials, query or fragment"
        );
        let same_origin =
            |other: &str| url::Url::parse(other).is_ok_and(|u| u.origin() == target.origin());
        let token = std::env::var("RALLY_TOKEN")
            .ok()
            .filter(|_| same_origin(&default_url))
            .or_else(|| {
                c["token"]
                    .as_str()
                    .filter(|_| same_origin(saved))
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let headers = headers::Headers::load(&c, &target)?;
        Ok(Self {
            url: url.to_owned(),
            token,
            headers: std::cell::RefCell::new(headers),
            http: reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 10 {
                        attempt.error("Too many Rally redirects")
                    } else if attempt
                        .previous()
                        .first()
                        .is_some_and(|u| u.origin() == attempt.url().origin())
                    {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .timeout(std::time::Duration::from_secs(180))
                .build()?,
        })
    }
    pub fn send(&self, path: &str, d: &Value) -> Result<Value> {
        let mut request = self
            .http
            .post(format!("{}/{path}", self.url))
            .headers(self.headers.borrow_mut().resolve(false)?);
        if !self.token.is_empty() {
            request = request.bearer_auth(&self.token);
        }
        let r = request
            .json(d)
            .send()
            .with_context(|| format!("Connecting to {}", self.url))?;
        let status = r.status();
        if status.is_redirection() {
            bail!(
                "HTTP {status}: Rally redirected to another origin. Check the server URL and gateway credentials."
            );
        }
        let body: Value = r.json().with_context(|| format!("HTTP {status}: expected a Rally JSON response. Check the server URL and gateway credentials."))?;
        if !status.is_success() {
            bail!("{}", body["error"].as_str().unwrap_or(status.as_str()));
        }
        Ok(body)
    }
    pub fn api(&self, op: &str, d: &Value) -> Result<Value> {
        self.send(&format!("api/{op}"), d)
    }
}

const OPS: &[(&str, &str, &str)] = &[
    (
        "secrets.pull",
        "mission",
        "Replace the local mission environment and checksums",
    ),
    (
        "secrets.push",
        "mission overwrite",
        "Push edited keys and check for concurrent edits",
    ),
    (
        "resource.privacy",
        "target private invites confirm",
        "Manage invitations and visibility",
    ),
    ("hook.list", "", "List your webhooks"),
    ("hook.create", "name", "Create a webhook"),
    (
        "hook.show",
        "hook",
        "Show endpoint, senders and connected routines",
    ),
    ("hook.token", "hook name", "Issue a sender token"),
    ("hook.revoke", "hook token", "Revoke a sender token by ID"),
    ("hook.remove", "hook", "Remove a webhook"),
    (
        "routine.transfer-preview",
        "routine",
        "Preview webhook handover",
    ),
    (
        "routine.preview",
        "cron timezone",
        "Preview the next scheduled runs",
    ),
    (
        "models.settings",
        "model",
        "Read or update the workspace WIP model",
    ),
    ("topic.list", "", "List organisation topics"),
    ("topic.create", "slug name", "Create a topic"),
    (
        "mission.list",
        "topic status local",
        "List accessible or locally opened missions",
    ),
    ("mission.show", "mission", "Show a mission and its branches"),
    (
        "mission.create",
        "topic title description repos confirm",
        "Create a mission",
    ),
    (
        "mission.update",
        "mission title description status",
        "Edit mission details",
    ),
    ("status.list", "", "List workspace mission statuses"),
    ("status.create", "name color", "Create a workspace status"),
    (
        "status.update",
        "status name color",
        "Change a workspace status",
    ),
    ("status.remove", "status", "Remove an unused status"),
    (
        "account.role",
        "person role preview confirm",
        "Assign member or admin access",
    ),
    (
        "mission.fork",
        "mission title",
        "Start a separate mission from its repositories",
    ),
    ("group.list", "", "List groups"),
    ("group.create", "name", "Create a group"),
    ("group.add", "group person", "Add a user to a group"),
    (
        "group.remove",
        "group person preview confirm",
        "Remove a user from a group",
    ),
    ("member.list", "mission", "List mission members"),
    (
        "member.add",
        "mission principal confirm",
        "Grant access to the whole mission",
    ),
    (
        "member.remove",
        "mission principal preview confirm",
        "Remove a direct mission grant",
    ),
    ("account.invite", "email role", "Invite a local user"),
    ("account.service", "name", "Create a service user and token"),
    ("people.list", "", "List organisation principals"),
    ("token.create", "name", "Issue a personal API token"),
    ("token.list", "", "List personal tokens"),
    ("token.revoke", "id", "Revoke a token"),
    ("connection.list", "", "List your personal Git connections"),
    (
        "connection.create",
        "name host username token-stdin",
        "Save a Git connection and read its token securely",
    ),
    (
        "connection.update",
        "connection token-stdin",
        "Replace the access token",
    ),
    (
        "connection.usage",
        "connection",
        "Show repositories, routines and skills using a connection",
    ),
    (
        "connection.remove",
        "connection confirm",
        "Disconnect a Git account and pause its attachments",
    ),
    (
        "repo.takeover",
        "mission name connection confirm",
        "Take responsibility for hosted Git access",
    ),
    (
        "repo.withdraw",
        "mission name",
        "Withdraw your Git access from a repository",
    ),
    (
        "skill.reconnect",
        "skill connection",
        "Reconnect a Git skill using your access",
    ),
    ("routing.list", "", "List organisation Git URL rules"),
    (
        "routing.create",
        "source upstream scope",
        "Route a repository or prefix to a hosted Git destination",
    ),
    (
        "routing.update",
        "id source upstream scope",
        "Edit a Git URL rule",
    ),
    ("routing.remove", "id", "Remove a Git URL rule"),
    (
        "repo.requests",
        "mission",
        "List pending repository read requests",
    ),
    (
        "repo.attach",
        "mission remote connection confirm",
        "Attach a requested dependency read-only",
    ),
    (
        "repo.dismiss",
        "mission remote",
        "Dismiss a repository read request",
    ),
    ("repo.list", "mission", "List tracked branches"),
    ("repo.status", "mission", "Inspect hosted checkout state"),
    (
        "repo.add",
        "mission name remote branch connection confirm",
        "Track a Git branch",
    ),
    ("repo.remove", "mission name", "Remove a branch binding"),
    ("repo.push", "mission name", "Push the current branch"),
    (
        "repo.return",
        "mission name",
        "Return to the original branch after manual reconciliation",
    ),
    ("thread.list", "mission", "List conversations"),
    (
        "thread.create",
        "mission title",
        "Start a hosted conversation",
    ),
    ("thread.rename", "thread title", "Rename a thread"),
    (
        "thread.subscribe",
        "thread mode",
        "Choose participating, all or muted",
    ),
    ("thread.mute", "thread", "Mute ordinary thread alerts"),
    ("run.list", "mission", "List agent invocations"),
    ("run.show", "run", "Inspect a run and its questions"),
    ("run.follow", "run", "Follow a working agent"),
    ("run.steer", "run body refs", "Steer an active invocation"),
    ("run.stop", "run", "Stop an invocation"),
    (
        "run.answer",
        "run question answer",
        "Submit a structured answer",
    ),
    ("agent.list", "", "List roles"),
    ("agent.show", "agent", "Read a role"),
    (
        "agent.create",
        "name model effort instructions private",
        "Create an agent role",
    ),
    (
        "agent.update",
        "agent model effort instructions",
        "Update a role on its next invocation",
    ),
    ("skill.list", "", "List managed skills"),
    ("skill.show", "skill", "Read a skill"),
    (
        "skill.create",
        "name body description private",
        "Author a Markdown skill",
    ),
    (
        "skill.import",
        "name remote ref path private connection",
        "Continuously import a skill from a Git remote on the server",
    ),
    (
        "skill.edit",
        "skill body name description",
        "Edit a GUI-authored skill",
    ),
    (
        "skill.attach",
        "skill target confirm",
        "Attach a skill to an agent or mission",
    ),
    ("skill.detach", "skill target confirm", "Detach a skill"),
    ("routine.list", "", "List your routines"),
    (
        "routine.show",
        "routine",
        "Inspect a routine and recent jobs",
    ),
    (
        "routine.create",
        "name command triggers repos directory",
        "Create a cron or webhook routine",
    ),
    (
        "routine.update",
        "routine name command triggers repos directory",
        "Edit a routine",
    ),
    ("routine.enable", "routine", "Enable a routine"),
    ("routine.disable", "routine", "Disable a routine"),
    ("routine.trigger", "routine payload", "Trigger a routine"),
    (
        "routine.transfer",
        "routine person confirm handover",
        "Transfer ownership",
    ),
    (
        "inbox.settings",
        "settings",
        "Read or update account-wide push preferences using JSON",
    ),
    ("inbox.list", "", "Show notifications"),
    ("inbox.read", "id", "Mark one or all notifications as read"),
    ("whoami", "", "Show the current principal"),
    (
        "search",
        "query mission limit offset",
        "Search accessible conversations",
    ),
    ("read", "thread", "Read thread Markdown"),
    ("pull", "thread output", "Export Markdown messages as YAML"),
    (
        "post",
        "thread body mentions refs confirm key silent",
        "Post a message and optionally dispatch agents",
    ),
    (
        "open",
        "mission print-path",
        "Materialise a mission and register its root",
    ),
    ("path", "mission", "Print the local mission path"),
    ("sync", "mission", "Refresh branch states"),
    (
        "wip",
        "mission name thread summary fork",
        "Commit hosted edits, optionally on a recovery branch",
    ),
    (
        "login",
        "email password token url oidc no-browser",
        "Sign in to a server",
    ),
    ("logout", "", "End your session"),
    (
        "bootstrap",
        "email name password code",
        "Create the first organisation owner",
    ),
    (
        "register",
        "name email password code",
        "Accept a local invitation",
    ),
    (
        "config.set",
        "key value",
        "Set url or directory in this profile",
    ),
    ("shell-init", "shell", "Print shell integration"),
    ("hooks.install", "adapter", "Install capture hooks"),
    ("hooks.uninstall", "adapter", "Remove Rally capture hooks"),
    (
        "hooks.status",
        "adapter",
        "Inspect capture hook configuration",
    ),
    ("hooks.capture", "adapter", "Receive a native hook on stdin"),
    ("hooks.flush", "", "Retry pending captures"),
];

fn cli() -> Command {
    let mut root = Command::new("rally")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Git workspaces and shared conversation context")
        .subcommand_required(true)
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print machine-readable JSON"),
        );
    let mut groups = std::collections::BTreeMap::new();
    for &(op, keys, help) in OPS {
        let (group, name) = op.split_once('.').unwrap_or(("", op));
        let mut c = Command::new(name).about(help);
        let positional = match op {
            "open" | "path" | "mission.show" | "mission.update" | "mission.fork" => Some("mission"),
            "read" | "pull" => Some("thread"),
            "search" => Some("query"),
            "shell-init" => Some("shell"),
            "hooks.install" | "hooks.uninstall" | "hooks.status" | "hooks.capture" => {
                Some("adapter")
            }
            _ => None,
        };
        for key in keys.split_whitespace() {
            let mut arg = Arg::new(key).long(match key {
                "mentions" => "mention",
                "refs" => "link",
                k => k,
            });
            if [
                "silent",
                "preview",
                "token-stdin",
                "handover",
                "confirm",
                "fork",
                "local",
                "print-path",
                "oidc",
                "no-browser",
            ]
            .contains(&key)
            {
                arg = arg.action(ArgAction::SetTrue);
            }
            if ["mentions", "refs", "overwrite"].contains(&key) {
                arg = arg.action(ArgAction::Append);
            }
            c = c.arg(arg);
        }
        if let Some(label) = positional {
            c = c.arg(Arg::new("positional").value_name(label.to_uppercase()));
        }
        if keys
            .split_whitespace()
            .any(|k| ["body", "instructions", "description"].contains(&k))
        {
            c = c.arg(
                Arg::new("file")
                    .long("file")
                    .help("Read body, instructions or description from a file (or - for stdin)"),
            );
        }
        if group.is_empty() {
            root = root.subcommand(c);
        } else {
            let g = groups
                .entry(group)
                .or_insert_with(|| Command::new(group).subcommand_required(true));
            *g = std::mem::take(g).subcommand(c);
        }
    }
    root.subcommands(groups.into_values())
}
fn stdin() -> Result<String> {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s)?;
    Ok(s)
}
pub fn main() -> Result<()> {
    let mut argv = std::env::args();
    if argv.next().is_some_and(|p| {
        Path::new(&p)
            .file_name()
            .is_some_and(|f| f == "git-remote-rallygit")
    }) {
        return git_remote::serve(
            &argv.next().context("Missing Git remote name")?,
            &argv.next().context("Missing Git remote URL")?,
        );
    }
    let matches = cli().get_matches();
    let (name, m) = matches.subcommand().unwrap();
    let (op, m) = if let Some((sub, m)) = m.subcommand() {
        (format!("{name}.{sub}"), m)
    } else {
        (name.to_string(), m)
    };
    let keys = OPS.iter().find(|(k, _, _)| k == &op).unwrap().1;
    let mut d = json!({});
    for key in keys.split_whitespace() {
        if [
            "silent",
            "preview",
            "token-stdin",
            "handover",
            "confirm",
            "fork",
            "local",
            "print-path",
            "oidc",
            "no-browser",
        ]
        .contains(&key)
        {
            if m.get_flag(key) {
                d[key] = json!(true);
            }
        } else if ["mentions", "refs", "overwrite"].contains(&key) {
            if let Some(v) = m.get_many::<String>(key) {
                d[key] = json!(v.collect::<Vec<_>>());
            }
        } else if let Some(value) = m.get_one::<String>(key) {
            d[key] = if [
                "private", "invites", "triggers", "repos", "answer", "settings", "payload",
                "offset", "limit",
            ]
            .contains(&key)
            {
                serde_json::from_str(value)?
            } else {
                json!(value)
            };
        }
    }
    if let Ok(Some(pos)) = m.try_get_one::<String>("positional") {
        d[keys.split_whitespace().next().unwrap()] = json!(pos);
    }
    if let Ok(Some(file)) = m.try_get_one::<String>("file") {
        let key = keys
            .split_whitespace()
            .find(|k| ["body", "instructions", "description"].contains(k))
            .unwrap();
        d[key] = json!(if file == "-" {
            stdin()?
        } else {
            std::fs::read_to_string(file)?
        });
    }
    if op == "connection.create" {
        warn_http(opt(&d, "host"));
    }
    if matches!(op.as_str(), "routing.create" | "routing.update") {
        warn_http(opt(&d, "upstream"));
    }
    if matches!(op.as_str(), "connection.create" | "connection.update") {
        d["token"] = json!(if d["token-stdin"] == true {
            stdin()?.trim_end().to_owned()
        } else {
            secret_input("Git access token")?
        });
        d.as_object_mut().unwrap().remove("token-stdin");
    }
    if op == "hooks.capture" {
        return hooks::capture(field(&d, "adapter")?);
    }
    if op.starts_with("hooks.") && op != "hooks.flush" {
        return hooks::manage(&op, field(&d, "adapter")?);
    }
    let as_json = matches.get_flag("json");
    if op == "shell-init" {
        println!("{}", workspace::shell(opt(&d, "shell"))?);
        return Ok(());
    }
    if op == "config.set" {
        let key = field(&d, "key")?;
        if !["directory", "url"].contains(&key) {
            bail!("Expected directory or url");
        }
        let path = config_dir().join("config.json");
        let mut c = read_json(&path)?;
        if key == "url" && c["url"] != d["value"] {
            c.as_object_mut().unwrap().remove("token");
        }
        c[key] = d["value"].clone();
        write_json(&path, &c)?;
        return print_result(&json!({"saved": key}), as_json);
    }
    let client = Client::new(if op == "login" {
        d["url"].as_str()
    } else {
        None
    })?;
    let local = || workspace::current(&std::env::current_dir()?, &client.url);
    if d["mission"].is_null()
        && keys.split_whitespace().any(|k| k == "mission")
        && op != "search"
        && op != "mission.list"
    {
        d["mission"] = json!(std::env::var("RALLY_MISSION").ok().or(local()?));
    }
    if d["thread"].is_null() && keys.split_whitespace().any(|k| k == "thread") {
        d["thread"] = json!(
            std::env::var("RALLY_THREAD")
                .ok()
                .or(std::env::var("RALLY_MISSION").ok())
                .or(local()?)
        );
    }
    if matches!(
        op.as_str(),
        "mission.create"
            | "repo.add"
            | "repo.attach"
            | "repo.takeover"
            | "routine.create"
            | "routine.update"
            | "skill.import"
            | "skill.reconnect"
            | "connection.update"
    ) {
        warn_git_destinations(&client, &op, &d);
    }
    let result = match op.as_str() {
        "secrets.pull" | "secrets.push" => secrets::sync(&client, &op, &d)?,
        "open" => {
            let dir = workspace::open(&client, field(&d, "mission")?)?;
            if d["print-path"] == true {
                println!("{}", dir.display());
            } else {
                println!(
                    "Mission ready.\ncd -- {}\nEnable shell integration with rally shell-init for your shell.",
                    workspace::quote(&dir.to_string_lossy())
                );
            }
            return Ok(());
        }
        "path" => {
            println!(
                "{}",
                workspace::path(&client, field(&d, "mission")?)?.display()
            );
            return Ok(());
        }
        "mission.list" if d["local"] == true => workspace::registered(&client.url)?,
        "hooks.flush" => json!({"sent":hooks::flush(&client)?}),
        "login" | "bootstrap" | "register" => {
            let mut c = read_json(&config_dir().join("config.json"))?;
            let mut client = client;
            if op == "login" && io::stdin().is_terminal() {
                client.headers.borrow_mut().resolve(true)?;
            }
            if d["oidc"] == true {
                client.token = oidc::login(&client, d["no-browser"] == true)?;
            } else if let Some(token) = d["token"].as_str() {
                client.token = token.to_string();
                client.api("whoami", &json!({}))?;
            } else {
                if d["password"].is_null() {
                    d["password"] = json!(secret_input("Password")?);
                }
                d["cli"] = json!(true);
                let r = client.send(&format!("auth/{op}"), &d)?;
                client.token = field(&r, "token")?.into();
            }
            c["url"] = json!(client.url);
            c["token"] = json!(client.token);
            write_json(&config_dir().join("config.json"), &c)?;
            client.api("whoami", &json!({}))?
        }
        "logout" => {
            client.send("auth/logout", &json!({}))?;
            let p = config_dir().join("config.json");
            let mut c = read_json(&p)?;
            if c["token"].as_str() == Some(&client.token) {
                c.as_object_mut().unwrap().remove("token");
                write_json(&p, &c)?;
            }
            json!({"signed_out":true})
        }
        "read" => {
            let t = client.api("read", &d)?;
            if !as_json {
                println!("{}", markdown(&t)?);
                return Ok(());
            }
            t
        }
        "pull" => {
            export(&client, &d)?;
            return Ok(());
        }
        "sync" => {
            let result = client.api("repo.sync", &d)?;
            if workspace::path(&client, field(&d, "mission")?).is_ok() {
                workspace::open(&client, field(&d, "mission")?)?;
            }
            result
        }
        "repo.status" => client.api("repo.list", &d)?,
        "wip" => client.api("repo.wip", &d)?,
        "thread.mute" => {
            d["mode"] = json!("muted");
            client.api("thread.subscribe", &d)?
        }
        "run.follow" => {
            let mut printed = String::new();
            loop {
                let r = client.api("run.show", &d)?;
                let text = opt(&r, "partial");
                if let Some(rest) = text.strip_prefix(&printed) {
                    print!("{rest}");
                    io::stdout().flush()?;
                }
                printed = text.into();
                if !["working", "queued", "interrupted", "stopping", "continuing"]
                    .contains(&opt(&r, "status"))
                {
                    println!("\n{}", r["status"]);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            return Ok(());
        }
        _ => {
            if op == "post" && d["body"].is_null() {
                d["body"] = json!(stdin()?);
            }
            client.api(&op, &d)?
        }
    };
    print_result(&result, as_json)
}
fn warn_http(value: &str) {
    if let Ok(url) = url::Url::parse(value)
        && url.scheme() == "http"
    {
        eprintln!(
            "Warning: Unencrypted HTTP Git destination {}. Repository data and any credentials sent there can be intercepted.",
            url.origin().ascii_serialization()
        );
    }
}

fn warn_git_destinations(client: &Client, op: &str, d: &Value) {
    let mut repos = d["repos"].as_array().cloned().unwrap_or_default();
    if d["remote"].is_string() {
        repos.push(d.clone());
    }
    if op == "repo.takeover"
        && let Ok(list) = client.api("repo.list", d)
        && let Some(repo) = list
            .as_array()
            .and_then(|rs| rs.iter().find(|r| r["name"] == d["name"]))
    {
        repos.push(repo.clone());
    }
    if op == "skill.reconnect"
        && let Ok(skill) = client.api("skill.show", d)
    {
        repos.push(skill);
    }
    if op == "connection.update"
        && let Ok(list) = client.api("connection.list", &json!({}))
        && let Some(connection) = list
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["id"] == d["connection"]))
    {
        warn_http(opt(connection, "origin"));
    }
    for repo in repos {
        if let Some(remote) = repo["remote"].as_str()
            && let Ok(route) = client.api("routing.resolve", &json!({"remote":remote}))
        {
            warn_http(opt(&route, "upstream"));
        }
    }
}

fn secret_input(label: &str) -> Result<String> {
    eprint!("{label}: ");
    io::stderr().flush()?;
    let hidden = std::process::Command::new("stty")
        .arg("-echo")
        .status()
        .is_ok_and(|s| s.success());
    let mut line = String::new();
    let result = io::stdin().read_line(&mut line);
    if hidden {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    result?;
    Ok(line.trim_end().to_string())
}
fn print_result(value: &Value, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string(value)?);
    } else if let Some(rows) = value.as_array() {
        for row in rows {
            let key = row["reference"]
                .as_str()
                .or(row["id"].as_str())
                .or(row["slug"].as_str())
                .unwrap_or("");
            let name = row["title"]
                .as_str()
                .or(row["name"].as_str())
                .or(row["body"].as_str())
                .or(row["summary"].as_str())
                .unwrap_or("");
            println!("{key}\t{name}");
        }
    } else {
        print!("{}", serde_yaml_ng::to_string(value)?);
    }
    Ok(())
}
fn markdown(t: &Value) -> Result<String> {
    let mut text = format!(
        "# {} · {}\n",
        field(&t["thread"], "mission")?,
        field(&t["thread"], "title")?
    );
    for m in t["messages"].as_array().unwrap() {
        text += &format!(
            "\n## {} ({})\n\n{}\n",
            opt(m, "author"),
            opt(m, "source"),
            opt(m, "body")
        );
    }
    Ok(text)
}
fn transcript(t: &Value) -> Value {
    let messages=t["messages"].as_array().unwrap().iter().map(|m|{
        let mut row=json!({"source":format!("{} ({})",opt(m,"author"),opt(m,"source")),"message":m["body"]});
        if m["commits"].as_object().is_some_and(|c|!c.is_empty()){row["commits"]=m["commits"].clone();}row
    }).collect::<Vec<_>>();
    json!({"mission":t["thread"]["mission"],"thread":t["thread"]["title"],"messages":messages})
}
fn export(c: &Client, d: &Value) -> Result<()> {
    let reference = field(d, "thread")?;
    let single = reference.contains(':') || reference.starts_with("th_");
    let output = d["output"].as_str().map(PathBuf::from);
    if single {
        let yaml = serde_yaml_ng::to_string(&transcript(&c.api("read", d)?))?;
        if let Some(p) = output {
            std::fs::write(p, yaml)?;
        } else {
            print!("{yaml}");
        }
        return Ok(());
    }
    let m = c.api("mission.show", &json!({"mission":reference}))?;
    let mut threads = vec![];
    if let Some(p) = &output {
        std::fs::create_dir_all(p)?;
    }
    for t in m["threads"].as_array().unwrap() {
        let id = field(t, "id")?;
        let doc = transcript(&c.api("read", &json!({"thread":id}))?);
        if let Some(dir) = &output {
            std::fs::write(
                dir.join(format!("{id}.yaml")),
                serde_yaml_ng::to_string(&doc)?,
            )?;
        } else {
            threads.push(doc);
        }
    }
    let manifest = json!({"mission":m["id"],"title":m["title"],"description":m["description"],"repos":m["repos"].as_array().unwrap().iter().map(|r|json!({"name":r["name"],"remote":r["remote"],"branch":r["branch"]})).collect::<Vec<_>>()});
    if let Some(dir) = output {
        std::fs::write(
            dir.join("mission.yaml"),
            serde_yaml_ng::to_string(&manifest)?,
        )?;
    } else {
        print!(
            "{}",
            serde_yaml_ng::to_string(&json!({"mission":manifest,"threads":threads}))?
        );
    }
    Ok(())
}
