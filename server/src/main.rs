use anyhow::Result;
use clap::Parser;
use platypus::server::Server;
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    Server::bind(&args.bind)
        .getter(|key: &str| Ok(format!("value_for_{} {:?}", key, Instant::now())))
        .target(&args.target)
        .version(env!("CARGO_PKG_VERSION"))
        .run()
        .await
}
