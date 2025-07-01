use crate::protocol::{self, ParseError};
use anyhow::Result;
use std::error::Error;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;
use tokio::time::Duration;
use tower::Service as TowerService;

pub struct Server {
    listen_address: String,
    notify_shutdown: Arc<Notify>,
}

impl Server {
    /// Creates a new Server instance bound to the specified listen address.
    ///
    /// # Arguments
    /// * `listen_address` - The address to bind the server to (e.g., "127.0.0.1:11212")
    ///
    /// # Returns
    /// A new Server instance ready for configuration
    pub fn bind(listen_address: &str) -> Self {
        Self {
            listen_address: listen_address.to_owned(),
            notify_shutdown: Arc::new(Notify::new()),
        }
    }

    /// Starts the memcached server and handles incoming connections.
    ///
    /// This method starts the TCP server, sets up signal handling for graceful shutdown,
    /// and processes memcached protocol commands from clients using the provided Service.
    /// It will run until a shutdown signal (SIGINT or SIGTERM) is received.
    ///
    /// # Returns
    /// Result<()> - Ok(()) on successful shutdown, Err on startup or runtime errors
    ///
    /// # Errors
    /// Returns an error if:
    /// - The TCP listener cannot bind to the specified address
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
        let listener = TcpListener::bind(self.listen_address.clone()).await?;

        // Trigger shutdown on Ctrl+C
        let notify_shutdown_on_ctrl_c = self.notify_shutdown.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            println!("Shutting down");
            notify_shutdown_on_ctrl_c.notify_waiters();
        });

        // Trigger shutdown on Linux TERM signal
        let notify_shutdown_on_term = self.notify_shutdown.clone();
        tokio::spawn(async move {
            if let Ok(mut term_signal) = signal(SignalKind::terminate()) {
                term_signal.recv().await.unwrap();
                println!("Shutting down");
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

                _ = monitor_interval.tick() => {
                    // Monitor tasks are now handled by the service internally
                }

                Ok((socket, _)) = listener.accept() => {
                    let notify = self.notify_shutdown.clone();
                    let service = service.clone();

                    tokio::spawn(async move {
                        let (read_half, write_half) = socket.into_split();
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
                                            let mut service = service.lock().await;
                                            match service.call(command_context).await {
                                                Ok(response) => {
                                                    // Handle quit command specially
                                                    if matches!(response, protocol::Response::Error(ref msg) if msg == "Connection should close") {
                                                        break;
                                                    }
                                                    let response_data = response.serialize(&protocol::ProtocolType::Text);
                                                    writer.write_all(&response_data).await.unwrap();
                                                }
                                                Err(e) => {
                                                    println!("Service call error: {}", e);
                                                    let error_response = protocol::Response::Error(e.to_string());
                                                    let response_data = error_response.serialize(&protocol::ProtocolType::Text);
                                                    writer.write_all(&response_data).await.unwrap();
                                                }
                                            }
                                        }
                                        Err(e) if matches!(e.downcast_ref::<ParseError>(), Some(ParseError::NoCommand)) => {}
                                        Err(e) => {
                                            println!("Parse error: {}", e);
                                            let error_response = protocol::Response::Error("Parse error".to_string());
                                            let response_data = error_response.serialize(&protocol::ProtocolType::Text);
                                            writer.write_all(&response_data).await.unwrap();
                                        }
                                    }
                                }
                            }
                            line.clear();
                        }
                    });
                }
            }
        }

        Ok(())
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            listen_address: self.listen_address.clone(),
            notify_shutdown: self.notify_shutdown.clone(),
        }
    }
}
