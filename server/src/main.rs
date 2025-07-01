use anyhow::{Result, anyhow};
use clap::Parser;
use platypus::server::Server;
use std::pin::Pin;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Address to bind to
    #[arg(short, long, default_value = "127.0.0.1:11212")]
    bind: String,

    /// Target memcached server
    #[arg(short, long, default_value = "memcache://127.0.0.1:11213")]
    target: String,
}

fn get_value(key: &str) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>> {
    let key = key.to_string();
    Box::pin(async move {
        match key.as_str() {
            "ok" => Ok(format!("value_for_{} {:?}", key, Instant::now())),
            _ => Err(anyhow!("invalid")),
        }
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    Server::bind(&args.bind)
        .getter(get_value)
        .target(&args.target)
        .version(env!("CARGO_PKG_VERSION"))
        .run()
        .await
}
