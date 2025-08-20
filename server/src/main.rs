use anyhow::{Result, anyhow};
use clap::Parser;
use platypus::{MonitorTasks, Router, Server, Service, source};
use std::time::Duration;
use tokio::time::Instant;
use tower::ServiceBuilder;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
use config::Config;

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

    /// Configuration file path
    #[arg(short, long)]
    config: Option<String>,
}

fn build_router_from_config(_: &Config) -> Result<Router> {
    // TODO: Implement
    Ok(Router::new())
}

#[tokio::main]
async fn main() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let pid = std::process::id();

    let args = Args::parse();

    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "debug".into());

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

    // Load configuration if provided
    let config = if let Some(config_path) = &args.config {
        Some(Config::from_file(config_path)?)
    } else {
        None
    };

    // Determine target from config or CLI args (CLI takes precedence)
    let target = if let Some(ref cfg) = config {
        if let Some(ref server_config) = cfg.server {
            if let Some(ref target_config) = server_config.target {
                target_config.to_url()
            } else {
                args.target.clone()
            }
        } else {
            args.target.clone()
        }
    } else {
        args.target.clone()
    };

    let monitor_tasks = MonitorTasks::new();
    let monitor_tasks_for_tick = monitor_tasks.clone();

    // Build router from config or use default routes
    let router = if let Some(ref cfg) = config {
        build_router_from_config(cfg)?
    } else {
        Router::new()
            .route(
                "test_(?<instance>.*)",
                source(|key| async move {
                    Some(format!("test {key} at {:?}", Instant::now()).to_string())
                })
                .with_ttl(Duration::from_secs(5))
                .with_expiry(Duration::from_secs(30))
                .with_box(),
            )
            .route(
                "other_(?<instance>.*)",
                source(|key| async move {
                    Some(format!("other {key} at {:?}", Instant::now()).to_string())
                })
                .with_ttl(Duration::from_secs(5))
                .with_expiry(Duration::from_secs(30))
                .with_box(),
            )
    };

    let handler = Service::with_monitor_tasks(monitor_tasks)
        .router(router)
        .target(&target)
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
