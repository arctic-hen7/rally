#[path = "../server/mod.rs"]
mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::serve().await
}
