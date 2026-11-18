use super::*;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

pub async fn call(app: &App, a: &Actor, op: &str, d: &Value) -> Result<Value> {
    personal(a)?;
    match op {
        "inbox.list"=> rows(&app.db,"SELECT to_jsonb(n)||jsonb_build_object('mission',t.mission,'title',t.title) FROM notifications n JOIN threads t ON t.id=n.thread WHERE n.person=$1 AND can_read($1,t.mission) ORDER BY n.id DESC LIMIT 200",&[&a.uid]).await,
        "inbox.read"=>{
            execute(&app.db,"UPDATE notifications SET seen=true WHERE person=$1 AND ($2='' OR id::text=$2)",&[&a.uid,opt(d,"id")]).await?;Ok(json!({"ok":true}))
        }
        "device.key"=>Ok(json!({"key":std::env::var("RALLY_VAPID_PUBLIC").ok()})),
        "device.subscribe"=>{
            let s=&d["subscription"];
            let sub:SubscriptionInfo=serde_json::from_value(s.clone())?;
            let url=url::Url::parse(&sub.endpoint)?;
            let host=url.host_str().unwrap_or("");
            if url.scheme()!="https" || !var("RALLY_PUSH_HOSTS","fcm.googleapis.com,updates.push.services.mozilla.com,web.push.apple.com").split(',').any(|h|host==h || host.ends_with(&format!(".{h}"))){bail!("Unsupported push service host");}
            sqlx::query("INSERT INTO devices VALUES($1,$2,$3) ON CONFLICT(endpoint) DO UPDATE SET person=excluded.person,subscription=excluded.subscription").bind(sub.endpoint).bind(&a.uid).bind(s).execute(&app.db).await?;
            Ok(json!({"ok":true}))
        }
        "device.unsubscribe"=>{
            execute(&app.db,"DELETE FROM devices WHERE person=$1 AND endpoint=$2",&[&a.uid,field(d,"endpoint")?]).await?;Ok(json!({"ok":true}))
        }
        _=>bail!("Unknown notification operation")
    }
}

pub async fn deliver(app: &App) -> Result<()> {
    let Ok(key) = std::env::var("RALLY_VAPID_PRIVATE") else {
        return Ok(());
    };
    let jobs=sqlx::query("SELECT n.id,n.summary,n.thread,n.run,d.endpoint,d.subscription FROM notifications n JOIN threads t ON t.id=n.thread JOIN devices d ON d.person=n.person WHERE NOT n.seen AND can_read(n.person,t.mission) AND n.created>now()-interval '1 day' AND NOT EXISTS(SELECT 1 FROM push_deliveries p WHERE p.notification=n.id AND p.endpoint=d.endpoint) ORDER BY n.id LIMIT 32").fetch_all(&app.db).await?;
    let client = IsahcWebPushClient::new()?;
    for job in jobs {
        let item: SubscriptionInfo = serde_json::from_value(job.get("subscription"))?;
        let mut signer = VapidSignatureBuilder::from_base64(&key, &item)?;
        signer.add_claim("sub", var("RALLY_VAPID_CONTACT", "mailto:admin@localhost"));
        let mut builder = WebPushMessageBuilder::new(&item);
        builder.set_vapid_signature(signer.build()?);
        let payload=json!({"title":"Rally","body":job.get::<String,_>("summary"),"thread":job.get::<String,_>("thread"),"run":job.get::<Option<String>,_>("run")}).to_string();
        builder.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
        match client.send(builder.build()?).await {
            Ok(()) => {
                sqlx::query("INSERT INTO push_deliveries VALUES($1,$2) ON CONFLICT DO NOTHING")
                    .bind(job.get::<i64, _>("id"))
                    .bind(&item.endpoint)
                    .execute(&app.db)
                    .await?;
            }
            Err(
                web_push::WebPushError::EndpointNotValid(_)
                | web_push::WebPushError::EndpointNotFound(_),
            ) => {
                execute(
                    &app.db,
                    "DELETE FROM devices WHERE endpoint=$1",
                    &[&item.endpoint],
                )
                .await?;
            }
            Err(e) => eprintln!("Push delivery: {e}"),
        }
    }
    Ok(())
}
