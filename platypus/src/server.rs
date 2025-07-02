use crate::protocol::{self, ParseError};
use anyhow::Result;
use std::error::Error;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;
use tokio::time::Duration;
use tower::Service as TowerService;
use tracing::{error, info, warn};

pub enum SocketType {
    Tcp(String),
    Unix(String),
}

pub struct Server {
    socket_config: SocketType,
    notify_shutdown: Arc<Notify>,
}

impl Server {
    /// Creates a new Server instance bound to the specified TCP address.
    ///
    /// # Arguments
    /// * `listen_address` - The TCP address to bind the server to (e.g., "127.0.0.1:11212")
    ///
    /// # Returns
    /// A new Server instance ready for configuration
    pub fn bind(listen_address: &str) -> Self {
        Self {
            socket_config: SocketType::Tcp(listen_address.to_owned()),
            notify_shutdown: Arc::new(Notify::new()),
        }
    }

    /// Creates a new Server instance bound to the specified Unix socket path.
    ///
    /// # Arguments
    /// * `socket_path` - The Unix socket path to bind the server to (e.g., "/tmp/platypus.sock")
    ///
    /// # Returns
    /// A new Server instance ready for configuration
    pub fn bind_unix(socket_path: &str) -> Self {
        Self {
            socket_config: SocketType::Unix(socket_path.to_owned()),
            notify_shutdown: Arc::new(Notify::new()),
        }
    }

    /// Starts the memcached server and handles incoming connections.
    ///
    /// This method starts the server (TCP or Unix socket), sets up signal handling for graceful shutdown,
    /// and processes memcached protocol commands from clients using the provided Service.
    /// It will run until a shutdown signal (SIGINT or SIGTERM) is received.
    ///
    /// # Returns
    /// Result<()> - Ok(()) on successful shutdown, Err on startup or runtime errors
    ///
    /// # Errors
    /// Returns an error if:
    /// - The listener cannot bind to the specified address or socket path
    /// - Network I/O errors occur during operation
    pub async fn serve<S>(self, service: S) -> Result<()>
    where
        S: TowerService<
                protocol::CommandContext,
                Response = protocol::Response,
                Error = Box<dyn Error + Send + Sync>,
            > + Clone
            + Send
            + 'static,
        S::Future: Send,
    {
        // Create appropriate listener based on socket type
        let (tcp_listener, unix_listener) = match &self.socket_config {
            SocketType::Tcp(addr) => {
                info!("Starting TCP server on {}", addr);
                (Some(TcpListener::bind(addr).await?), None)
            }
            SocketType::Unix(path) => {
                info!("Starting Unix socket server on {}", path);
                // Remove existing socket file if it exists
                let _ = std::fs::remove_file(path);
                (None, Some(UnixListener::bind(path)?))
            }
        };

        // Trigger shutdown on Ctrl+C
        let notify_shutdown_on_ctrl_c = self.notify_shutdown.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            warn!("Shutting down");
            notify_shutdown_on_ctrl_c.notify_waiters();
        });

        // Trigger shutdown on Linux TERM signal
        let notify_shutdown_on_term = self.notify_shutdown.clone();
        tokio::spawn(async move {
            if let Ok(mut term_signal) = signal(SignalKind::terminate()) {
                term_signal.recv().await.unwrap();
                warn!("Shutting down");
                notify_shutdown_on_term.notify_waiters();
            }
        });

        let mut monitor_interval = tokio::time::interval(Duration::from_secs(1));
        let service = Arc::new(tokio::sync::Mutex::new(service));

        loop {
            tokio::select! {
                _ = self.notify_shutdown.notified() => {
                    break;
                }

                // Handle TCP connections
                Ok((socket, _)) = async {
                    match &tcp_listener {
                        Some(listener) => listener.accept().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let notify = self.notify_shutdown.clone();
                    let service = service.clone();

                    tokio::spawn(async move {
                        let (read_half, write_half) = socket.into_split();
                        Self::handle_connection(read_half, write_half, notify, service).await;
                    });
                }

                // Handle Unix socket connections
                Ok((socket, _)) = async {
                    match &unix_listener {
                        Some(listener) => listener.accept().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let notify = self.notify_shutdown.clone();
                    let service = service.clone();

                    tokio::spawn(async move {
                        let (read_half, write_half) = socket.into_split();
                        Self::handle_connection(read_half, write_half, notify, service).await;
                    });
                }
            }
        }

        Ok(())
    }

    async fn handle_connection<R, W, S>(
        read_half: R,
        write_half: W,
        notify: Arc<Notify>,
        service: Arc<tokio::sync::Mutex<S>>,
    ) where
        R: tokio::io::AsyncRead + Unpin,
        W: tokio::io::AsyncWrite + Unpin,
        S: TowerService<
                protocol::CommandContext,
                Response = protocol::Response,
                Error = Box<dyn Error + Send + Sync>,
            > + Clone
            + Send
            + 'static,
        S::Future: Send,
    {
        let mut reader = BufReader::new(read_half);
        let mut writer = write_half;
        let mut line = String::new();

        loop {
            tokio::select! {
                _ = notify.notified() => {
                    break;
                }

                data = protocol::recv_command(&mut reader) => {
                    match data {
                        Ok(command_context) => {
                            let protocol = command_context.protocol.clone();
                            let mut service = service.lock().await;
                            match service.call(command_context).await {
                                Ok(response) => {
                                    // Handle quit command specially
                                    if matches!(response, protocol::Response::Error(ref msg) if msg == "Connection should close") {
                                        break;
                                    }
                                    let response_data = response.serialize(&protocol);
                                    _ = writer.write_all(&response_data).await;
                                }
                                Err(e) => {
                                    error!(error = %e, "Service call error");
                                    let error_response = protocol::Response::Error(e.to_string());
                                    let response_data = error_response.serialize(&protocol);
                                    _ = writer.write_all(&response_data).await;
                                }
                            }
                        }
                        Err(e) if matches!(e.downcast_ref::<ParseError>(), Some(ParseError::NoCommand)) => {}
                        Err(e) => {
                            error!(error = %e, "Parse error");
                            let error_response = protocol::Response::Error("Parse error".to_string());
                            let response_data = error_response.serialize(&protocol::ProtocolType::Text);
                            _ = writer.write_all(&response_data).await;
                        }
                    }
                }
            }
            line.clear();
        }
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            socket_config: self.socket_config.clone(),
            notify_shutdown: self.notify_shutdown.clone(),
        }
    }
}

impl Clone for SocketType {
    fn clone(&self) -> Self {
        match self {
            SocketType::Tcp(addr) => SocketType::Tcp(addr.clone()),
            SocketType::Unix(path) => SocketType::Unix(path.clone()),
        }
    }
}
