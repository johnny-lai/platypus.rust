use anyhow::Result;
use platypus::server::Server;

#[tokio::main]
async fn main() -> Result<()> {
    Server::bind("127.0.0.1:11212")
        .getter(|key: &str| format!("value_for_{}", key))
        .target("memcache://127.0.0.1:11213")
        .run()
        .await
}
