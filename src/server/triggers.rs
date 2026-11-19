use super::*;
use chrono::{DateTime, Utc};
use cron::Schedule;
use sqlx::{Postgres, Transaction};
use std::str::FromStr;

pub fn upcoming(expr: &str, zone: &str) -> Result<Vec<DateTime<Utc>>> {
    let timezone = zone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| anyhow::anyhow!("Unknown IANA time zone"))?;
    let expr = if expr.split_whitespace().count() == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    };
    let dates: Vec<_> = Schedule::from_str(&expr)?
        .upcoming(timezone)
        .take(3)
        .map(|d| d.to_utc())
        .collect();
    if dates.is_empty() {
        bail!("Schedule has no future occurrences");
    }
    Ok(dates)
}

pub async fn webhook(
    State(app): State<App>,
    Path(hook): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> ApiResult {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let mut tx = app.db.begin().await?;
    let sender: Option<String> = sqlx::query_scalar(
        "UPDATE webhook_tokens SET last_used=now() WHERE hook=$1 AND digest=$2 RETURNING id",
    )
    .bind(&hook)
    .bind(rally_teams::digest(bearer))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(sender) = sender else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    let delivery = ["idempotency-key", "x-gitea-delivery", "x-github-delivery"]
        .iter()
        .find_map(|k| headers.get(*k)?.to_str().ok());
    if let Some(key) = delivery {
        let result=sqlx::query("INSERT INTO webhook_deliveries(hook,sender,delivery) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(&hook).bind(sender).bind(key).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            tx.commit().await?;
            return Ok((StatusCode::ACCEPTED, Json(json!({"duplicate":true}))).into_response());
        }
    }
    let mut jobs = vec![];
    for row in sqlx::query("SELECT to_jsonb(r) AS definition FROM routines r JOIN routine_triggers t ON t.routine=r.id JOIN webhooks w ON w.id=t.webhook AND w.owner_id=r.owner_id WHERE t.webhook=$1 AND r.enabled FOR SHARE OF r,w").bind(hook).fetch_all(&mut *tx).await?{
        jobs.push(enqueue(&mut tx,&row.get("definition"),&payload).await?);
    }
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"jobs":jobs}))).into_response())
}

pub async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    r: &Value,
    payload: &Value,
) -> Result<String> {
    if r["enabled"] != true {
        bail!("Routine disabled");
    }
    let jid = id("job");
    sqlx::query("INSERT INTO jobs(id,routine,owner_id,payload,status,definition) VALUES($1,$2,$3,$4,'queued',$5)").bind(&jid).bind(field(r,"id")?).bind(field(r,"owner_id")?).bind(payload).bind(r).execute(&mut **tx).await?;
    Ok(jid)
}

pub async fn manage(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    if op == "hook.create" {
        let hook = id("hook");
        execute(
            &app.db,
            "INSERT INTO webhooks(id,owner_id,name) VALUES($1,$2,$3)",
            &[&hook, &a.uid, field(d, "name")?],
        )
        .await?;
        return Ok(json!({"id":hook,"url":format!("{}/hooks/{hook}",app.base)}));
    }
    if op == "hook.list" {
        return rows(
            &app.db,
            "SELECT to_jsonb(w) FROM webhooks w WHERE owner_id=$1 ORDER BY name",
            &[&a.uid],
        )
        .await;
    }
    let hook = field(d, "hook")?;
    let mut tx = app.db.begin().await?;
    let mut w: Value = sqlx::query_scalar(
        "SELECT to_jsonb(w) FROM webhooks w WHERE id=$1 AND owner_id=$2 FOR UPDATE",
    )
    .bind(hook)
    .bind(&a.uid)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Access denied"))?;
    let result = match op {
        "hook.show" => {
            w["url"] = json!(format!("{}/hooks/{hook}", app.base));
            w["tokens"] = json!(
                sqlx::query_scalar::<_, Value>(
                    "SELECT to_jsonb(t)-'digest' FROM webhook_tokens t WHERE hook=$1 ORDER BY name"
                )
                .bind(hook)
                .fetch_all(&mut *tx)
                .await?
            );
            w["routines"]=json!(sqlx::query_scalar::<_,Value>("SELECT jsonb_build_object('id',r.id,'name',r.name) FROM routines r JOIN routine_triggers t ON t.routine=r.id WHERE t.webhook=$1 ORDER BY name").bind(hook).fetch_all(&mut *tx).await?);
            w
        }
        "hook.token" => {
            let token = rally_teams::secret();
            let tid = id("ht");
            sqlx::query("INSERT INTO webhook_tokens(id,hook,name,digest) VALUES($1,$2,$3,$4)")
                .bind(&tid)
                .bind(hook)
                .bind(field(d, "name")?)
                .bind(rally_teams::digest(&token))
                .execute(&mut *tx)
                .await?;
            json!({"id":tid,"token":token})
        }
        "hook.revoke" => {
            sqlx::query("DELETE FROM webhook_tokens WHERE hook=$1 AND id=$2")
                .bind(hook)
                .bind(field(d, "token")?)
                .execute(&mut *tx)
                .await?;
            json!({"ok":true})
        }
        "hook.remove" => {
            sqlx::query("DELETE FROM webhooks WHERE id=$1")
                .bind(hook)
                .execute(&mut *tx)
                .await?;
            json!({"ok":true})
        }
        _ => bail!("Unknown webhook operation"),
    };
    tx.commit().await?;
    Ok(result)
}

pub async fn replace(
    tx: &mut Transaction<'_, Postgres>,
    rid: &str,
    uid: &str,
    triggers: &Value,
) -> Result<()> {
    let list = triggers
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected a list of triggers"))?;
    sqlx::query("DELETE FROM routine_triggers WHERE routine=$1")
        .bind(rid)
        .execute(&mut **tx)
        .await?;
    for t in list {
        if let Some(hook) = t["webhook"].as_str() {
            let ok = sqlx::query("SELECT id FROM webhooks WHERE id=$1 AND owner_id=$2 FOR SHARE")
                .bind(hook)
                .bind(uid)
                .fetch_optional(&mut **tx)
                .await?
                .is_some();
            if !ok {
                bail!("Access denied");
            }
            sqlx::query("INSERT INTO routine_triggers(id,routine,webhook) VALUES($1,$2,$3)")
                .bind(id("tr"))
                .bind(rid)
                .bind(hook)
                .execute(&mut **tx)
                .await?;
        } else {
            let cron = field(t, "cron")?;
            let zone = t["timezone"].as_str().unwrap_or("UTC");
            let next = upcoming(cron, zone)?[0];
            sqlx::query("INSERT INTO routine_triggers(id,routine,cron,timezone,next_at) VALUES($1,$2,$3,$4,$5)").bind(id("tr")).bind(rid).bind(cron).bind(zone).bind(next).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

pub async fn schedule(app: &App) -> Result<()> {
    let mut tx = app.db.begin().await?;
    for row in sqlx::query("SELECT t.id,t.cron,t.timezone,to_jsonb(r) AS definition FROM routine_triggers t JOIN routines r ON r.id=t.routine WHERE r.enabled AND t.next_at<=now() FOR UPDATE OF t,r SKIP LOCKED").fetch_all(&mut *tx).await?{
        enqueue(&mut tx,&row.get("definition"),&json!({})).await?;
        let next=upcoming(row.get("cron"),row.get("timezone")).ok().and_then(|v|v.first().copied());
        sqlx::query("UPDATE routine_triggers SET next_at=$2 WHERE id=$1").bind(row.get::<String,_>("id")).bind(next).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
