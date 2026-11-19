use super::*;

pub async fn manager(app: &App, a: &Actor, target: &str) -> Result<()> {
    personal(a)?;
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM library_resources WHERE id=$1 AND (creator=$2 OR $3))",
    )
    .bind(target)
    .bind(&a.uid)
    .bind(a.admin || a.owner)
    .fetch_one(&app.db)
    .await?;
    if !allowed {
        bail!("Access denied");
    }
    Ok(())
}

pub async fn privacy(app: &App, a: &Actor, d: &Value) -> Result<Value> {
    let target = field(d, "target")?;
    manager(app, a, target).await?;
    let mut tx = app.db.begin().await?;
    let table = if target.starts_with("a_") {
        "agents"
    } else {
        "skills"
    };
    let old: bool = sqlx::query_scalar(&format!(
        "SELECT private FROM {table} WHERE id=$1 FOR UPDATE"
    ))
    .bind(target)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(private) = d["private"].as_bool() {
        if old && !private && table == "agents" && d["confirm"] != true {
            let attached:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments a JOIN skills s ON s.id=a.skill WHERE a.target=$1 AND s.private)").bind(target).fetch_one(&mut *tx).await?;
            if attached {
                bail!(
                    "Confirm that everyone can use the attached private skills and their secrets through this agent"
                );
            }
        }
        sqlx::query(&format!("UPDATE {table} SET private=$2 WHERE id=$1"))
            .bind(target)
            .bind(private)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(list) = d.get("invites") {
        let list = list
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected a list of people or group IDs"))?;
        sqlx::query("DELETE FROM resource_invites WHERE resource=$1")
            .bind(target)
            .execute(&mut *tx)
            .await?;
        for p in list {
            let principal = p
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid principal"))?;
            let inserted=sqlx::query("INSERT INTO resource_invites SELECT $1,$2 WHERE EXISTS(SELECT id FROM people WHERE id=$2 UNION ALL SELECT id FROM groups WHERE id=$2)").bind(target).bind(principal).execute(&mut *tx).await?;
            if inserted.rows_affected() != 1 {
                bail!("Unknown principal");
            }
        }
    }
    let invites: Vec<String> = sqlx::query_scalar(
        "SELECT principal FROM resource_invites WHERE resource=$1 ORDER BY principal",
    )
    .bind(target)
    .fetch_all(&mut *tx)
    .await?;
    let targets:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',target,'name',coalesce(m.title,a.name,target)) FROM (SELECT target FROM attachments WHERE skill=$1 UNION SELECT mission FROM members WHERE principal=$1) ids LEFT JOIN missions m ON m.id=target LEFT JOIN agents a ON a.id=target ORDER BY target").bind(target).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(json!({"private":d["private"].as_bool().unwrap_or(old),"invites":invites,"targets":targets}))
}
