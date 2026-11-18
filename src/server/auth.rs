use super::*;
use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString, rand_core::OsRng},
};
use axum::{
    extract::Query,
    http::header::{COOKIE, SET_COOKIE},
    response::Redirect,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use rally_teams::{digest, secret};
use sha2::{Digest, Sha256};

fn cookie<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| {
            let (k, v) = c.trim().split_once('=')?;
            (k == key).then_some(v)
        })
}
fn set_cookie(app: &App, r: &mut Response, name: &str, value: &str, age: i64) {
    let secure = if app.base.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    r.headers_mut().append(
        SET_COOKIE,
        format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{secure}")
            .parse()
            .unwrap(),
    );
}

pub async fn actor(app: &App, headers: &HeaderMap) -> Result<Actor> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| cookie(headers, "rally_session"))
        .ok_or_else(|| anyhow::anyhow!("Sign in required"))?;
    let hash = digest(token);
    let row=sqlx::query("SELECT p.id,p.name,p.owner,NULL::text AS run,NULL::text AS routine FROM tokens t JOIN people p ON p.id=t.person WHERE t.digest=$1 AND (t.expires IS NULL OR t.expires>now()) UNION ALL SELECT p.id,a.name,false,r.id,NULL FROM runs r JOIN people p ON p.id=r.dispatcher JOIN sessions s ON s.id=r.session JOIN agents a ON a.id=s.agent WHERE r.token_digest=$1 AND r.status='working' UNION ALL SELECT p.id,t.name,p.owner,NULL,t.id FROM jobs j JOIN people p ON p.id=j.owner_id JOIN routines t ON t.id=j.routine WHERE j.token_digest=$1 AND j.status='working'").bind(hash).fetch_optional(&app.db).await?.ok_or_else(||anyhow::anyhow!("Sign in required"))?;
    Ok(Actor {
        uid: row.get("id"),
        name: row.get("name"),
        owner: row.get("owner"),
        run: row.get("run"),
        routine: row.get("routine"),
    })
}

async fn login(app: &App, uid: &str, cli: bool) -> ApiResult {
    let token = secret();
    execute(&app.db,"INSERT INTO tokens(digest,person,label,expires) VALUES($1,$2,'Session',now()+interval '30 days')",&[&digest(&token),uid]).await?;
    let mut response = Json(if cli {
        json!({"token":token})
    } else {
        json!({"signed_in":true})
    })
    .into_response();
    set_cookie(app, &mut response, "rally_session", &token, 30 * 86400);
    Ok(response)
}

pub async fn endpoint(
    State(app): State<App>,
    Path(op): Path<String>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> ApiResult {
    let cli = data["cli"] == true;
    match op.as_str() {
        "status" => {
            let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM people)")
                .fetch_one(&app.db)
                .await?;
            Ok(
                Json(json!({"setup":!ready,"oidc":std::env::var("RALLY_OIDC_ISSUER").is_ok()}))
                    .into_response(),
            )
        }
        "bootstrap" | "register" => {
            let email = field(&data, "email")?.trim().to_lowercase();
            let password = field(&data, "password")?.to_string();
            if password.len() < 10 || password.len() > 1024 {
                return Err(
                    anyhow::anyhow!("Use a password between 10 and 1024 characters").into(),
                );
            }
            let hash = tokio::task::spawn_blocking(move || {
                Argon2::default()
                    .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
                    .map(|h| h.to_string())
                    .map_err(|_| anyhow::anyhow!("Password hashing failed"))
            })
            .await??;
            let mut tx = app.db.begin().await?;
            let owner = op == "bootstrap";
            if owner {
                sqlx::query("SELECT pg_advisory_xact_lock(712735)")
                    .execute(&mut *tx)
                    .await?;
                if sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM people)")
                    .fetch_one(&mut *tx)
                    .await?
                    || std::env::var("RALLY_BOOTSTRAP_SECRET")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .as_deref()
                        != Some(field(&data, "code")?)
                {
                    return Err(anyhow::anyhow!("Access denied").into());
                }
            } else if sqlx::query(
                "DELETE FROM invites WHERE digest=$1 AND lower(email)=$2 AND expires>now()",
            )
            .bind(digest(field(&data, "code")?))
            .bind(&email)
            .execute(&mut *tx)
            .await?
            .rows_affected()
                != 1
            {
                return Err(anyhow::anyhow!("Invalid invitation").into());
            }
            let uid = id("u");
            sqlx::query("INSERT INTO people(id,name,email,password,owner) VALUES($1,$2,$3,$4,$5)")
                .bind(&uid)
                .bind(field(&data, "name")?)
                .bind(email)
                .bind(hash)
                .bind(owner)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            login(&app, &uid, cli).await
        }
        "login" => {
            let row=sqlx::query("SELECT id,password FROM people WHERE email=lower($1) AND password IS NOT NULL AND kind='user'").bind(field(&data,"email")?).fetch_optional(&app.db).await?;
            let password = field(&data, "password")?.to_string();
            let (uid, hash) = row
                .map(|r| (r.get::<String, _>("id"), r.get::<String, _>("password")))
                .ok_or_else(|| anyhow::anyhow!("Invalid credentials"))?;
            if !tokio::task::spawn_blocking(move || {
                PasswordHash::new(&hash).is_ok_and(|h| {
                    Argon2::default()
                        .verify_password(password.as_bytes(), &h)
                        .is_ok()
                })
            })
            .await?
            {
                return Err(anyhow::anyhow!("Invalid credentials").into());
            }
            login(&app, &uid, cli).await
        }
        "logout" => {
            if let Some(t) = cookie(&headers, "rally_session").or_else(|| {
                headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
            }) {
                execute(&app.db, "DELETE FROM tokens WHERE digest=$1", &[&digest(t)]).await?;
            }
            let mut res = Json(json!({})).into_response();
            set_cookie(&app, &mut res, "rally_session", "", 0);
            Ok(res)
        }
        _ => Err(anyhow::anyhow!("Not found").into()),
    }
}

async fn discovery(app: &App) -> Result<Value> {
    let issuer = std::env::var("RALLY_OIDC_ISSUER")?;
    let doc: Value = app
        .http
        .get(format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if doc["issuer"] != issuer {
        bail!("OIDC issuer mismatch");
    }
    Ok(doc)
}
pub async fn oidc_start(State(app): State<App>) -> ApiResult {
    let doc = discovery(&app).await?;
    let state = secret();
    let nonce = secret();
    let verifier = secret();
    execute(&app.db,"INSERT INTO oidc_flows(state,nonce,verifier,expires) VALUES($1,$2,$3,now()+interval '10 minutes')",&[&state,&nonce,&verifier]).await?;
    let mut url = url::Url::parse(field(&doc, "authorization_endpoint")?)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", std::env::var("RALLY_OIDC_CLIENT_ID")?),
        ("redirect_uri", format!("{}/auth/oidc/callback", app.base)),
        ("scope", "openid profile email".into()),
        ("response_type", "code".into()),
        ("state", state.clone()),
        ("nonce", nonce),
        ("code_challenge_method", "S256".into()),
        (
            "code_challenge",
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        ),
    ]);
    let mut r = Redirect::to(url.as_str()).into_response();
    set_cookie(&app, &mut r, "rally_oidc", &state, 600);
    Ok(r)
}
pub async fn oidc_callback(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let state = q
        .get("state")
        .ok_or_else(|| anyhow::anyhow!("Missing state"))?;
    if cookie(&headers, "rally_oidc") != Some(state) {
        return Err(anyhow::anyhow!("Access denied").into());
    }
    let flow = sqlx::query(
        "DELETE FROM oidc_flows WHERE state=$1 AND expires>now() RETURNING nonce,verifier",
    )
    .bind(state)
    .fetch_one(&app.db)
    .await?;
    let doc = discovery(&app).await?;
    let client = std::env::var("RALLY_OIDC_CLIENT_ID")?;
    let mut request = app.http.post(field(&doc, "token_endpoint")?);
    let mut form = vec![
        ("code", q.get("code").cloned().unwrap_or_default()),
        ("code_verifier", flow.get("verifier")),
        ("redirect_uri", format!("{}/auth/oidc/callback", app.base)),
        ("grant_type", "authorization_code".into()),
    ];
    if let Ok(secret) = std::env::var("RALLY_OIDC_CLIENT_SECRET") {
        request = request.basic_auth(&client, Some(secret));
    } else {
        form.push(("client_id", client.clone()));
    }
    let tokens: Value = request
        .form(&form)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let jwt = field(&tokens, "id_token")?;
    let header = decode_header(jwt)?;
    let keys: JwkSet = app
        .http
        .get(field(&doc, "jwks_uri")?)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let key = keys
        .find(header.kid.as_deref().unwrap_or(""))
        .ok_or_else(|| anyhow::anyhow!("Unknown signing key"))?;
    let mut check = Validation::new(Algorithm::RS256);
    check.set_audience(&[&client]);
    check.set_issuer(&[field(&doc, "issuer")?]);
    let claims = decode::<Value>(jwt, &DecodingKey::from_jwk(key)?, &check)?.claims;
    if claims["nonce"] != flow.get::<String, _>("nonce")
        || claims.get("azp").is_some_and(|v| v != &client)
    {
        return Err(anyhow::anyhow!("Access denied").into());
    }
    let subject = format!("{}|{}", field(&doc, "issuer")?, field(&claims, "sub")?);
    let name = claims["name"]
        .as_str()
        .or(claims["preferred_username"].as_str())
        .unwrap_or(field(&claims, "sub")?);
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM people WHERE owner)")
        .fetch_one(&app.db)
        .await?
    {
        return Err(anyhow::anyhow!("Bootstrap the organisation owner first").into());
    }
    let uid:String=sqlx::query_scalar("INSERT INTO people(id,name,subject) VALUES($1,$2,$3) ON CONFLICT(subject) DO UPDATE SET name=excluded.name RETURNING id").bind(id("u")).bind(name).bind(subject).fetch_one(&app.db).await?;
    let session = login(&app, &uid, false).await?;
    let mut r = Redirect::to("/").into_response();
    if let Some(c) = session.headers().get(SET_COOKIE) {
        r.headers_mut().append(SET_COOKIE, c.clone());
    }
    set_cookie(&app, &mut r, "rally_oidc", "", 0);
    Ok(r)
}
