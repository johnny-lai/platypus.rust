use anyhow::{Result, anyhow};
use clap::Parser;
use platypus::{MonitorTasks, Server, Service};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};
use tower::ServiceBuilder;
use tracing::info;
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

    /// Log format: json or text
    #[arg(long, default_value = "text")]
    log_format: String,
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
    let version = env!("CARGO_PKG_VERSION");
    let pid = std::process::id();

    let args = Args::parse();

    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    match args.log_format.as_str() {
        "json" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_file(false)
                        .with_line_number(false)
                        .with_target(true)
                        .with_thread_ids(true)
                        .with_thread_names(true)
                        .json()
                        .flatten_event(true)
                        .with_span_events(
                            tracing_subscriber::fmt::format::FmtSpan::ENTER
                                | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
                        ),
                )
                .init();
        }
        "text" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_file(false)
                        .with_line_number(false)
                        .with_target(true)
                        .with_thread_ids(true)
                        .with_thread_names(true)
                        .with_span_events(
                            tracing_subscriber::fmt::format::FmtSpan::ENTER
                                | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
                        ),
                )
                .init();
        }
        _ => {
            return Err(anyhow!(
                "Invalid log format '{}'. Valid options are 'json' or 'text'",
                args.log_format
            ));
        }
    }

    // Create a span that includes version and PID for all subsequent logs
    let span = tracing::info_span!("application", version = %version, pid = %pid);
    let _guard = span.enter();

    info!("Server starting");

    let monitor_tasks = MonitorTasks::new();
    let monitor_tasks_for_tick = monitor_tasks.clone();

    let handler = Service::with_monitor_tasks(monitor_tasks)
        .getter(get_value)
        .target(&args.target)
        .version(env!("CARGO_PKG_VERSION"));

    // Keep a reference to the original service for shutdown
    let handler_for_shutdown = handler.clone();

    let service = ServiceBuilder::new()
        .timeout(Duration::from_secs(5))
        .service(handler);

    let mut server = match (args.bind, args.unix_socket) {
        (Some(bind_addr), None) => Server::bind(&bind_addr),
        (None, Some(unix_path)) => Server::bind_unix(&unix_path),
        (None, None) => Server::bind("127.0.0.1:11212"), // Default TCP binding
        (Some(_), Some(_)) => return Err(anyhow!("Cannot specify both --bind and --unix-socket")),
    };

    server.with_monitor_tasks(monitor_tasks_for_tick);
    let _ = server.serve(service).await?;

    // Shutdown the service to ensure Writer threads are properly joined
    handler_for_shutdown.shutdown();

    info!("Server terminated");
    Ok(())
}
