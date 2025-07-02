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
    /// Address to bind to (TCP address like "127.0.0.1:11212")
    #[arg(short, long, conflicts_with = "unix_socket")]
    bind: Option<String>,

    /// Unix socket path to bind to (e.g., "/tmp/platypus.sock")
    #[arg(short, long, conflicts_with = "bind")]
    unix_socket: Option<String>,

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

    let server = match (args.bind, args.unix_socket) {
        (Some(bind_addr), None) => Server::bind(&bind_addr),
        (None, Some(unix_path)) => Server::bind_unix(&unix_path),
        (None, None) => Server::bind("127.0.0.1:11212"), // Default TCP binding
        (Some(_), Some(_)) => return Err(anyhow!("Cannot specify both --bind and --unix-socket")),
    };

    server.serve(service).await
}
