use crate::monitor::MonitorTasks;
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

#[derive(Clone)]
pub enum SocketType {
    Tcp(String),
    Unix(String),
}

pub struct Server {
    socket_config: SocketType,
    notify_shutdown: Arc<Notify>,
    monitor_tasks: Option<MonitorTasks<String>>,
}

impl Server {
    /// Creates a new Server instance bound to the specified listen address.
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
            monitor_tasks: None,
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
            monitor_tasks: None,
        }
    }

    pub fn with_monitor_tasks(&mut self, monitor_tasks: MonitorTasks<String>) -> &mut Self {
        self.monitor_tasks = Some(monitor_tasks);
        self
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

                _ = monitor_interval.tick() => {
                    if let Some(ref monitor_tasks) = self.monitor_tasks {
                        monitor_tasks.tick().await;
                    }
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
                            line.clear();
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
            monitor_tasks: self.monitor_tasks.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Command, CommandContext, ProtocolType, Response};
    use std::pin::Pin;
    use std::task::{Context, Poll};

    #[derive(Clone)]
    struct MockService {
        should_error: bool,
    }

    impl TowerService<CommandContext> for MockService {
        type Response = Response;
        type Error = Box<dyn Error + Send + Sync>;
        type Future =
            Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: CommandContext) -> Self::Future {
            let should_error = self.should_error;
            Box::pin(async move {
                if should_error {
                    return Err("Mock service error".into());
                }
                match req.command {
                    Command::Version => Ok(Response::Version("1.0.0".to_string())),
                    Command::Quit => Ok(Response::Error("Connection should close".to_string())),
                    Command::Get(_keys) => Ok(Response::Values(vec![])),
                    _ => Ok(Response::Error("Unsupported command".to_string())),
                }
            })
        }
    }

    #[test]
    fn test_server_bind() {
        let server = Server::bind("127.0.0.1:11211");
        match server.socket_config {
            SocketType::Tcp(addr) => assert_eq!(addr, "127.0.0.1:11211"),
            _ => panic!("Expected TCP socket type"),
        }
        assert!(server.monitor_tasks.is_none());
    }

    #[test]
    fn test_server_bind_unix() {
        let server = Server::bind_unix("/tmp/test.sock");
        match server.socket_config {
            SocketType::Unix(path) => assert_eq!(path, "/tmp/test.sock"),
            _ => panic!("Expected Unix socket type"),
        }
        assert!(server.monitor_tasks.is_none());
    }

    #[test]
    fn test_server_with_monitor_tasks() {
        let mut server = Server::bind("127.0.0.1:11211");
        let monitor_tasks = MonitorTasks::new();

        server.with_monitor_tasks(monitor_tasks);
        assert!(server.monitor_tasks.is_some());
    }

    #[test]
    fn test_server_clone() {
        let server = Server::bind("127.0.0.1:11211");
        let cloned_server = server.clone();

        match (&server.socket_config, &cloned_server.socket_config) {
            (SocketType::Tcp(addr1), SocketType::Tcp(addr2)) => {
                assert_eq!(addr1, addr2);
            }
            _ => panic!("Socket types don't match"),
        }
    }

    #[test]
    fn test_socket_type_clone() {
        let tcp_socket = SocketType::Tcp("127.0.0.1:11211".to_string());
        let cloned_tcp = tcp_socket.clone();
        match (tcp_socket, cloned_tcp) {
            (SocketType::Tcp(addr1), SocketType::Tcp(addr2)) => {
                assert_eq!(addr1, addr2);
            }
            _ => panic!("TCP socket types don't match"),
        }

        let unix_socket = SocketType::Unix("/tmp/test.sock".to_string());
        let cloned_unix = unix_socket.clone();
        match (unix_socket, cloned_unix) {
            (SocketType::Unix(path1), SocketType::Unix(path2)) => {
                assert_eq!(path1, path2);
            }
            _ => panic!("Unix socket types don't match"),
        }
    }

    #[tokio::test]
    async fn test_server_bind_invalid_address() {
        let server = Server::bind("invalid_address");
        let service = MockService {
            should_error: false,
        };

        let result = server.serve(service).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_server_bind_unix_invalid_path() {
        let server = Server::bind_unix("/invalid/path/that/does/not/exist/test.sock");
        let service = MockService {
            should_error: false,
        };

        let result = server.serve(service).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_server_configuration_chaining() {
        let mut server = Server::bind("127.0.0.1:11211");
        let monitor_tasks = MonitorTasks::new();

        server.with_monitor_tasks(monitor_tasks);

        match server.socket_config {
            SocketType::Tcp(addr) => assert_eq!(addr, "127.0.0.1:11211"),
            _ => panic!("Expected TCP socket type"),
        }
        assert!(server.monitor_tasks.is_some());
    }

    #[tokio::test]
    async fn test_mock_service_version_command() {
        let mut service = MockService {
            should_error: false,
        };
        let command_context = CommandContext {
            command: Command::Version,
            protocol: ProtocolType::Text,
        };

        let result = service.call(command_context).await.unwrap();
        match result {
            Response::Version(version) => assert_eq!(version, "1.0.0"),
            _ => panic!("Expected Version response"),
        }
    }

    #[tokio::test]
    async fn test_mock_service_quit_command() {
        let mut service = MockService {
            should_error: false,
        };
        let command_context = CommandContext {
            command: Command::Quit,
            protocol: ProtocolType::Text,
        };

        let result = service.call(command_context).await.unwrap();
        match result {
            Response::Error(msg) => assert_eq!(msg, "Connection should close"),
            _ => panic!("Expected Error response"),
        }
    }

    #[tokio::test]
    async fn test_mock_service_error() {
        let mut service = MockService { should_error: true };
        let command_context = CommandContext {
            command: Command::Version,
            protocol: ProtocolType::Text,
        };

        let result = service.call(command_context).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Mock service error");
    }

    #[tokio::test]
    async fn test_mock_service_poll_ready() {
        let mut service = MockService {
            should_error: false,
        };
        let mut context = Context::from_waker(futures::task::noop_waker_ref());

        let result = service.poll_ready(&mut context);
        assert!(matches!(result, Poll::Ready(Ok(()))));
    }

    #[tokio::test]
    async fn test_mock_service_get_command() {
        let mut service = MockService {
            should_error: false,
        };
        let command_context = CommandContext {
            command: Command::Get(vec!["test_key".to_string()]),
            protocol: ProtocolType::Text,
        };

        let result = service.call(command_context).await.unwrap();
        match result {
            Response::Values(items) => assert_eq!(items.len(), 0),
            _ => panic!("Expected Values response"),
        }
    }
}
