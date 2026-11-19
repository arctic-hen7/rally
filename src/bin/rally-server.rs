#[path = "../server/mod.rs"]
mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::provider::start().await
}
