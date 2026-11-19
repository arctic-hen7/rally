use super::*;
use anyhow::Context;
use rand::RngCore;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use sqlx::PgConnection;
use std::path::Path;

pub fn load_key(root: &Path, has_secrets: bool) -> Result<LessSafeKey> {
    let path = root.join("keys/secrets.key");
    if !path.try_exists()? {
        if has_secrets {
            bail!("Restore keys/secrets.key from the installation backup before startup");
        }
        let mut bytes = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        super::provider::write_private(&path, &bytes)?;
    }
    let key = UnboundKey::new(&AES_256_GCM, &std::fs::read(path)?)
        .map_err(|_| anyhow::anyhow!("Invalid installation secret key"))?;
    Ok(LessSafeKey::new(key))
}
pub(super) fn encrypt(app: &App, target: &str, key: &str, value: &str) -> Result<Vec<u8>> {
    let mut nonce = [0; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut ciphertext = value.as_bytes().to_vec();
    app.secret_key
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(format!("{target}/{key}")),
            &mut ciphertext,
        )
        .map_err(|_| anyhow::anyhow!("Could not encrypt secret"))?;
    let mut out = nonce.to_vec();
    out.extend(ciphertext);
    Ok(out)
}
pub(super) fn decrypt(app: &App, target: &str, key: &str, buf: &mut [u8]) -> Result<String> {
    let (nonce, value) = buf
        .split_at_mut_checked(12)
        .context("Damaged encrypted secret")?;
    let bytes = app
        .secret_key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce.try_into()?),
            Aad::from(format!("{target}/{key}")),
            value,
        )
        .map_err(|_| anyhow::anyhow!("Secret authentication failed"))?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

async fn resolved(db: &mut PgConnection, app: &App, target: &str) -> Result<Value> {
    let mut out = json!({});
    for row in sqlx::query("SELECT s.*,coalesce(l.name,s.target) AS label FROM scoped_secrets s LEFT JOIN library_resources l ON l.id=s.target WHERE target=$1 OR target IN(SELECT skill FROM attachments WHERE target=$1) ORDER BY (s.target=$1),s.target,s.name").bind(target).fetch_all(db).await?{
        let key:String=row.get("name");let origin:String=row.get("target");
        let entry=&mut out[&key];
        let previous=entry.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut sources=previous.clone();sources.push(json!({"id":origin,"name":row.get::<String,_>("label")}));
        if origin!=target && !previous.is_empty(){
            *entry=json!({"sources":sources,"conflict":true,"checksum":null});continue;
        }
        let value=decrypt(app,&origin,&key,&mut row.get::<Vec<u8>,_>("ciphertext"))?;
        *entry=json!({"value":value,"source":origin,"sources":sources,"checksum":row.get::<String,_>("checksum")});
    }
    Ok(out)
}

pub async fn environment(
    app: &App,
    mid: &str,
    agent: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let mut tx = app.db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let mut entries = resolved(&mut tx, app, mid).await?;
    if let Some(a) = agent {
        for (k, v) in resolved(&mut tx, app, a).await?.as_object().unwrap() {
            entries[k] = v.clone();
        }
    }
    tx.commit().await?;
    entries
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            Ok((
                k.clone(),
                v["value"]
                    .as_str()
                    .with_context(|| {
                        format!("Resolve duplicate skill secret {k} on the mission or agent")
                    })?
                    .to_owned(),
            ))
        })
        .collect()
}

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    let raw = d["target"]
        .as_str()
        .or(d["mission"].as_str())
        .context("Choose a target")?;
    let target = raw.trim_start_matches('#');
    if target.starts_with("a_") || target.starts_with("sk_") {
        access::manager(app, a, target).await?;
    } else {
        mission(app, a, target, true).await?;
        one(
            &app.db,
            "SELECT to_jsonb(id) FROM missions WHERE id=$1",
            &[target],
        )
        .await?;
    }
    let mut tx = app.db.begin().await?;
    if op == "secrets.apply" {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
    }
    let current = resolved(&mut tx, app, target).await?;
    if op == "secrets.list" {
        return Ok(current);
    }
    if op != "secrets.apply" {
        bail!("Unknown secrets operation");
    }
    let changes = d["changes"].as_object().context("Expected changed keys")?;
    let mut conflicts = vec![];
    for (key, change) in changes {
        if !rally_teams::env_key(key) {
            bail!("Invalid or reserved environment key {key}");
        }
        let old = &current[key];
        if change.get("base").is_none()
            || change["base"] != old["checksum"]
            || change["source"] != old["source"]
        {
            conflicts.push(key.as_str());
        }
    }
    if !conflicts.is_empty() {
        bail!(
            "Secrets changed upstream: {}. Pull or explicitly reconcile these keys before pushing.",
            conflicts.join(", ")
        );
    }
    for (key, change) in changes {
        match change.get("value") {
            Some(Value::String(value)) if !value.contains('\0') => {
                let encrypted = encrypt(app, target, key, value)?;
                sqlx::query("INSERT INTO scoped_secrets VALUES($1,$2,$3,$4) ON CONFLICT(target,name) DO UPDATE SET ciphertext=excluded.ciphertext,checksum=excluded.checksum").bind(target).bind(key).bind(encrypted).bind(rally_teams::digest(value)).execute(&mut *tx).await?;
            }
            Some(Value::Null) => {
                if current[key]["source"] != target {
                    bail!("Remove inherited {key} on its source skill or detach the skill");
                }
                sqlx::query("DELETE FROM scoped_secrets WHERE target=$1 AND name=$2")
                    .bind(target)
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
            }
            _ => bail!("Invalid value for {key}"),
        }
    }
    let result = resolved(&mut tx, app, target).await?;
    tx.commit().await?;
    Ok(result)
}
