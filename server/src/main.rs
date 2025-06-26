use anyhow::Result;
use platypus::server::Server;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    Server::bind("127.0.0.1:11212")
        .getter(|key: &str| Ok(format!("value_for_{} {:?}", key, Instant::now())))
        .target("memcache://127.0.0.1:11213")
        .run()
        .await
}
