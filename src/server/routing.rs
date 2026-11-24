use super::*;
use anyhow::ensure;
use std::collections::BTreeMap;
use url::Url;

// Reject ambiguous path components before URL parsing can normalize them
fn safe_path(path: &str) -> bool {
    path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.~".contains(&c))
    })
}

fn parse(input: &str) -> Result<Url> {
    ensure!(
        input.len() <= 2048
            && !input.contains(['?', '#', '%', '\\'])
            && !input.chars().any(|c| c.is_whitespace() || c.is_control()),
        "Invalid Git URL"
    );
    let expanded = if input.contains("://") {
        input.to_owned()
    } else if let Some((host, path)) = input.split_once(':').filter(|(h, p)| {
        !h.contains(['/', '['])
            && (h.contains('@')
                || !p
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .bytes()
                    .all(|c| c.is_ascii_digit()))
    }) {
        format!("ssh://{host}/{}", path.trim_start_matches('/'))
    } else {
        format!("https://{input}")
    };
    let raw_path = expanded
        .split_once("://")
        .unwrap()
        .1
        .split_once('/')
        .map(|(_, p)| p.trim_end_matches('/'))
        .unwrap_or("");
    ensure!(
        raw_path.is_empty() || safe_path(raw_path),
        "Use a Git URL with plain path components"
    );
    let url = Url::parse(&expanded)?;
    ensure!(
        matches!(url.scheme(), "ssh" | "https")
            && url.host().is_some()
            && url.password().is_none()
            && (url.username().is_empty()
                || (url.scheme() == "ssh"
                    && url
                        .username()
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)))),
        "Use an HTTPS or SSH Git URL without credentials"
    );
    Ok(url)
}

fn stem(s: &str) -> &str {
    let s = s.trim_end_matches('/');
    s.strip_suffix(".git").unwrap_or(s)
}

fn aliases(source: &Url, include_ssh: bool) -> Vec<String> {
    let mut list = vec![source.as_str().trim_end_matches('/').to_owned()];
    let host = source.host().unwrap().to_string();
    let path = source.path().trim_matches('/');
    if source.scheme() == "https" && source.port().is_none() {
        list.push(
            format!("https://{host}:443/{path}")
                .trim_end_matches('/')
                .to_owned(),
        );
    }
    if (source.scheme() == "ssh" && source.port().is_none_or(|p| p == 22))
        || (source.scheme() == "https" && include_ssh && source.port().is_none())
    {
        let user = if source.scheme() == "https" {
            "git"
        } else {
            source.username()
        };
        let login = if user.is_empty() {
            host
        } else {
            format!("{user}@{host}")
        };
        list.push(format!("{login}:{path}"));
        list.push(format!("{login}:/{path}"));
        for port in ["", ":22"] {
            list.push(
                format!("ssh://{login}{port}/{path}")
                    .trim_end_matches('/')
                    .to_owned(),
            );
        }
    }
    list
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    admin(a)?;
    match op {
        "routing.list" => {
            rows(
                &app.db,
                "SELECT to_jsonb(r) FROM git_routes r ORDER BY source",
                &[],
            )
            .await
        }
        "routing.remove" => {
            execute(
                &app.db,
                "DELETE FROM git_routes WHERE id=$1",
                &[field(d, "id")?],
            )
            .await?;
            Ok(json!({"ok":true}))
        }
        "routing.create" | "routing.update" => {
            let source = parse(field(d, "source")?.trim())?;
            let upstream = d
                .get("upstream")
                .map(|_| field(d, "upstream"))
                .transpose()?
                .unwrap_or(source.as_str())
                .trim();
            connections::origin(upstream)?;
            let upstream = parse(upstream)?;
            let scope = field(d, "scope")?;
            ensure!(
                scope == "prefix" || scope == "repository",
                "Choose prefix or repository"
            );
            ensure!(
                scope == "prefix"
                    || (!source.path().trim_matches('/').is_empty()
                        && !upstream.path().trim_matches('/').is_empty()),
                "Enter the repository path"
            );
            let id = if op == "routing.create" {
                id("route")
            } else {
                field(d, "id")?.into()
            };
            let mut tx = app.db.begin().await?;
            sqlx::query("LOCK TABLE git_routes IN SHARE ROW EXCLUSIVE MODE")
                .execute(&mut *tx)
                .await?;
            let rules = sqlx::query("SELECT id,source,upstream,scope FROM git_routes WHERE id<>$1")
                .bind(&id)
                .fetch_all(&mut *tx)
                .await?;
            let mut map = BTreeMap::new();
            for row in rules {
                add_rule(
                    &mut map,
                    &row.get::<String, _>("source"),
                    &row.get::<String, _>("upstream"),
                    &row.get::<String, _>("scope"),
                )?;
            }
            add_rule(&mut map, source.as_str(), upstream.as_str(), scope)?;
            let sql = if op == "routing.create" {
                "INSERT INTO git_routes(id,source,upstream,scope) VALUES($1,$2,$3,$4)"
            } else {
                "UPDATE git_routes SET source=$2,upstream=$3,scope=$4 WHERE id=$1"
            };
            let changed = sqlx::query(sql)
                .bind(&id)
                .bind(source.as_str())
                .bind(upstream.as_str())
                .bind(scope)
                .execute(&mut *tx)
                .await?;
            ensure!(changed.rows_affected() == 1, "Unknown routing rule");
            tx.commit().await?;
            Ok(json!({"id":id}))
        }
        _ => bail!("Unknown routing operation"),
    }
}

// The key is a literal Git insteadOf prefix. Prefix rules end at a path separator
type Routes = BTreeMap<String, (String, bool)>;
fn add_rule(routes: &mut Routes, source: &str, upstream: &str, scope: &str) -> Result<()> {
    let prefix = scope == "prefix";
    let source = parse(source)?;
    let target = parse(upstream)?;
    let dest = target.as_str().trim_end_matches('/');
    for alias in aliases(&source, true)
        .into_iter()
        .chain(aliases(&target, false))
    {
        let key = if prefix {
            if alias.ends_with(':') {
                alias
            } else {
                format!("{}/", alias.trim_end_matches('/'))
            }
        } else {
            stem(&alias).to_owned()
        };
        let value = (dest.to_owned(), prefix);
        if let Some(old) = routes.insert(key, value.clone()) {
            ensure!(
                old == value,
                "This Git URL already maps to another HTTPS destination"
            );
        }
    }
    Ok(())
}

pub async fn worker_env(
    app: &App,
    lease: &connections::Lease,
    repos: &Value,
) -> Result<Vec<(String, String)>> {
    let mut routes = Routes::new();
    for row in sqlx::query("SELECT source,upstream,scope FROM git_routes")
        .fetch_all(&app.db)
        .await?
    {
        add_rule(
            &mut routes,
            &row.get::<String, _>("source"),
            &row.get::<String, _>("upstream"),
            &row.get::<String, _>("scope"),
        )?;
    }
    for repo in repos.as_array().into_iter().flatten() {
        let remote = field(repo, "remote")?;
        if let Ok(url) = parse(remote) {
            routes
                .entry(stem(url.as_str()).to_owned())
                .or_insert((url.to_string(), false));
        }
    }
    if routes.is_empty() {
        return Ok(lease.env());
    }
    let mut config: Vec<_> = routes
        .keys()
        .map(|source| {
            (
                format!("url.rallygit::{source}.insteadOf"),
                source.to_owned(),
            )
        })
        .collect();
    config.push(("protocol.rallygit.allow".into(), "always".into()));
    let mut env = lease.git_env(config);
    if engine::local() {
        env.push((
            "PATH".into(),
            format!(
                "{}:{}",
                app.root.join("tools").display(),
                std::env::var("PATH")?
            ),
        ));
    }
    env.push(("RALLY_GIT_ROUTES".into(), serde_json::to_string(&routes)?));
    env.push(("RALLY_GIT_PROXY".into(), lease.base.clone()));
    Ok(env)
}

pub fn requested(path: &str) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    ensure!(path.len() < 4096, "Invalid repository URL");
    let bytes = URL_SAFE_NO_PAD.decode(path)?;
    let remote = std::str::from_utf8(&bytes)?;
    connections::origin(remote)?;
    let url = parse(remote)?;
    ensure!(
        !url.path().trim_matches('/').is_empty(),
        "Missing repository path"
    );
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

pub fn identity(remote: &str) -> String {
    Url::parse(remote)
        .map(|u| stem(u.as_str()).to_owned())
        .unwrap_or_else(|_| remote.to_owned())
}

pub async fn request(app: &App, mid: &str, remote: &str) -> Result<()> {
    let mut tx = app.db.begin().await?;
    let key = identity(remote);
    let managed: bool = sqlx::query_scalar("SELECT git_route_covers($1)")
        .bind(&key)
        .fetch_one(&mut *tx)
        .await?;
    ensure!(managed, "No organisation routing rule for this repository");
    sqlx::query("SELECT id FROM missions WHERE id=$1 FOR UPDATE")
        .bind(mid)
        .execute(&mut *tx)
        .await?;
    let remotes: Vec<String> = sqlx::query_scalar("SELECT remote FROM repos WHERE mission=$1")
        .bind(mid)
        .fetch_all(&mut *tx)
        .await?;
    ensure!(
        !remotes.iter().any(|url| identity(url) == key),
        "Retry with the attached repository"
    );
    let inserted = sqlx::query(
        "INSERT INTO git_requests(mission,remote,identity) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(mid)
    .bind(remote)
    .bind(&key)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() > 0 {
        sqlx::query("INSERT INTO notifications(person,thread,reason,summary) SELECT p.id,t.id,'git-access',$2 FROM people p JOIN threads t ON t.mission=$1 AND t.kind='main' WHERE p.kind='user' AND can_read(p.id,$1) AND NOT EXISTS(SELECT 1 FROM subscriptions s WHERE s.thread=t.id AND s.person=p.id AND s.mode='muted')")
            .bind(mid).bind(format!("Git read access requested for {remote}")).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if inserted.rows_affected() > 0 {
        let _ = app.events.send(mid.into());
    }
    Ok(())
}

pub async fn pending(app: &App, mid: &str) -> Result<Value> {
    rows(&app.db,"SELECT to_jsonb(remote) FROM git_requests WHERE mission=$1 AND NOT dismissed AND git_route_covers(identity) ORDER BY remote",&[mid]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_ports_and_path_boundaries() -> Result<()> {
        let mut rules = Routes::new();
        add_rule(
            &mut rules,
            "git@forge.test:team/",
            "https://forge.test:8443/mirrors/team/",
            "prefix",
        )?;
        let target = ("https://forge.test:8443/mirrors/team".to_owned(), true);
        for key in [
            "ssh://git@forge.test/team/",
            "ssh://git@forge.test:22/team/",
            "git@forge.test:team/",
            "git@forge.test:/team/",
            "https://forge.test:8443/mirrors/team/",
        ] {
            assert_eq!(rules.get(key), Some(&target));
        }
        assert!(!rules.contains_key("git@forge.test:team"));
        let mut explicit = Routes::new();
        add_rule(
            &mut explicit,
            "ssh://git@forge.test:2222/team/shared.git",
            "https://forge.test/team/shared.git",
            "repository",
        )?;
        assert!(explicit.contains_key("ssh://git@forge.test:2222/team/shared"));
        assert!(!explicit.contains_key("git@forge.test:team/shared"));
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_upstream_paths() {
        for url in [
            "https://forge.test/team/../other",
            "https://forge.test/team/%2Fother",
            "https://forge.test/team//other",
            "https://forge.test/team/x?next=other",
            "https://user:token@forge.test/team/x",
        ] {
            assert!(parse(url).is_err());
        }
    }
}
