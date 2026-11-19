use super::*;
use std::collections::HashMap;

pub async fn index(app: &App) -> Result<()> {
    let Some(model) = &app.model else {
        return Ok(());
    };
    let pending=sqlx::query("SELECT m.id,(SELECT string_agg(body,E'\\n' ORDER BY id) FROM (SELECT body,id FROM messages WHERE thread=m.thread AND NOT silent AND id<=m.id ORDER BY id DESC LIMIT 3) passage) AS passage FROM messages m WHERE NOT silent AND embedding IS NULL ORDER BY id LIMIT 64").fetch_all(&app.db).await?;
    if pending.is_empty() {
        return Ok(());
    }
    let texts = pending
        .iter()
        .map(|r| r.get("passage"))
        .collect::<Vec<String>>();
    let model = model.clone();
    let vectors = tokio::task::spawn_blocking(move || model.encode(&texts)).await?;
    for (row, v) in pending.into_iter().zip(vectors) {
        sqlx::query("UPDATE messages SET embedding=$2 WHERE id=$1")
            .bind(row.get::<i64, _>("id"))
            .bind(v)
            .execute(&app.db)
            .await?;
    }
    Ok(())
}

pub async fn query(app: &App, a: &Actor, d: &Value) -> Result<Value> {
    let term = field(d, "query")?;
    let filter = opt(d, "mission").trim_start_matches('#');
    let allowed: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM missions WHERE ($2='' OR id=$2) AND (can_read($1,id) OR $3!='')",
    )
    .bind(&a.uid)
    .bind(filter)
    .bind(a.run.as_deref().unwrap_or(""))
    .fetch_all(&app.db)
    .await?;
    let mut scope = vec![];
    for mid in allowed {
        if a.run.is_none() || mission(app, a, &mid, false).await.is_ok() {
            scope.push(mid);
        }
    }
    let extra: Vec<String> = if let Some(rid) = &a.run {
        sqlx::query_scalar("SELECT jsonb_object_keys(grants) FROM runs WHERE id=$1")
            .bind(rid)
            .fetch_all(&app.db)
            .await?
    } else {
        vec![]
    };
    let query = "SELECT m.id,ts_rank_cd(to_tsvector('english',m.body),websearch_to_tsquery('english',$1)) AS rank FROM messages m JOIN threads t ON t.id=m.thread JOIN missions x ON x.id=t.mission WHERE NOT m.silent AND (t.mission=ANY($2) OR t.id=ANY($3)) AND ($4='' OR t.mission=$4) AND (to_tsvector('english',m.body||' '||x.title||' '||t.title) @@ websearch_to_tsquery('english',$1) OR strpos(lower(m.body),lower($1))>0) ORDER BY rank DESC,m.id DESC LIMIT 200";
    let lexical = sqlx::query(query)
        .bind(term)
        .bind(&scope)
        .bind(&extra)
        .bind(filter)
        .fetch_all(&app.db)
        .await?;
    let mut scores = HashMap::<i64, f64>::new();
    for (i, row) in lexical.iter().enumerate() {
        scores.insert(row.get("id"), 1.5 / (20 + i) as f64);
    }
    if let Some(model) = &app.model {
        let v = model.encode_single(term);
        let hits=sqlx::query("SELECT m.id,m.embedding FROM messages m JOIN threads t ON t.id=m.thread WHERE NOT m.silent AND (t.mission=ANY($1) OR t.id=ANY($2)) AND ($3='' OR t.mission=$3) AND m.embedding IS NOT NULL").bind(&scope).bind(&extra).bind(filter).fetch_all(&app.db).await?;
        let mut similarity = hits
            .iter()
            .map(|row| {
                let vector: Vec<f32> = row.get("embedding");
                (
                    row.get::<i64, _>("id"),
                    v.iter().zip(vector).map(|(a, b)| a * b).sum::<f32>(),
                )
            })
            .filter(|(_, s)| *s > 0.20)
            .collect::<Vec<_>>();
        similarity.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (i, (id, _)) in similarity.into_iter().take(200).enumerate() {
            *scores.entry(id).or_default() += 1.0 / (20 + i) as f64;
        }
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
    let offset = d["offset"].as_u64().unwrap_or(0) as usize;
    let ids = ranked
        .into_iter()
        .skip(offset)
        .take(d["limit"].as_u64().unwrap_or(30).min(100) as usize)
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let hits=sqlx::query_scalar::<_,Value>("SELECT jsonb_build_object('id',m.id,'body',m.body,'author',m.author,'source',m.source,'thread',t.id,'reference','#'||t.mission||':'||t.title,'mission',t.mission) FROM messages m JOIN threads t ON t.id=m.thread WHERE m.id=ANY($1) ORDER BY array_position($1,m.id)").bind(ids).fetch_all(&app.db).await?;
    Ok(json!(hits))
}
