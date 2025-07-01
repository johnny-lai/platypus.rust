use anyhow::{Result, anyhow};
use clap::Parser;
use platypus::{Server, Service};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};
use tower::ServiceBuilder;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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

fn get_value(key: &str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
    let key = key.to_string();
    Box::pin(async move {
        match key.as_str() {
            "ok" => Some(format!("value_for_{} {:?}", key, Instant::now())),
            _ => None,
        }
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    let service = Service::new()
        .getter(get_value)
        .target(&args.target)
        .version(env!("CARGO_PKG_VERSION"));

    let service = ServiceBuilder::new()
        .timeout(Duration::from_secs(5))
        .service(service);

    Server::bind(&args.bind).serve(service).await
}
