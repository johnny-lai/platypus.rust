use crate::protocol::{self, Command, Item, ParseError, Response};
use crate::{Error, MonitorTask, Writer};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, Notify};
use tokio::time::Duration;

pub struct Server<F> {
    getter: Option<Arc<F>>,
    listen_address: String,
    target_address: Option<String>,
    version: String,

    monitor_tasks: Arc<Mutex<HashMap<String, MonitorTask<String>>>>,
    notify_shutdown: Arc<Notify>,
}

impl<F> Server<F>
where
    F: Fn(&str) -> Result<String, Error> + Clone + Send + Sync + 'static,
{
    /// Creates a new Server instance bound to the specified listen address.
    ///
    /// # Arguments
    /// * `listen_address` - The address to bind the server to (e.g., "127.0.0.1:11212")
    ///
    /// # Returns
    /// A new Server instance ready for configuration
    pub fn bind(listen_address: &str) -> Self {
        Self {
            getter: None,
            listen_address: listen_address.to_owned(),
            target_address: None,
            version: "0.0.0".into(),
            monitor_tasks: Arc::new(Mutex::new(HashMap::new())),
            notify_shutdown: Arc::new(Notify::new()),
        }
    }

    /// Sets the getter function for custom value retrieval.
    ///
    /// The getter function is called when a key is requested and not found in cache.
    /// It should return a Result containing the value for the given key.
    ///
    /// # Arguments
    /// * `f` - A function that takes a key (&str) and returns Result<String, Error>
    ///
    /// # Returns
    /// Self for method chaining
    pub fn getter(mut self, f: F) -> Self {
        self.getter = Some(Arc::new(f));
        self
    }

    /// Sets the target memcached server address for operation forwarding.
    ///
    /// When set, certain operations may be forwarded to the target server.
    /// The address should be in the format "memcache://host:port".
    ///
    /// # Arguments
    /// * `target_address` - The target memcached server address
    ///
    /// # Returns
    /// Self for method chaining
    pub fn target(mut self, target_address: &str) -> Self {
        self.target_address = Some(target_address.to_owned());
        self
    }

    /// Sets the version that will be returned from the VERSION command.
    /// This defaults to "0.0.0".
    ///
    /// # Arguments
    /// * `version` - The target memcached server address
    ///
    /// # Returns
    /// Self for method chaining
    pub fn version(mut self, version: &str) -> Self {
        self.version = version.into();
        self
    }

    /// Starts the memcached server and handles incoming connections.
    ///
    /// This method starts the TCP server, sets up signal handling for graceful shutdown,
    /// and processes memcached protocol commands from clients. It will run until
    /// a shutdown signal (SIGINT or SIGTERM) is received.
    ///
    /// # Returns
    /// Result<()> - Ok(()) on successful shutdown, Err on startup or runtime errors
    ///
    /// # Errors
    /// Returns an error if:
    /// - The TCP listener cannot bind to the specified address
    /// - Network I/O errors occur during operation
    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::bind(self.listen_address.clone()).await?;
        let target = Some(Arc::new(Writer::<String>::new(
            self.target_address.clone().unwrap().as_ref(),
        )));

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

        let self_for_loop = self.clone();
        loop {
            tokio::select! {
                _ = self.notify_shutdown.notified() => {
                    break;
                }

                _ = monitor_interval.tick() => {
                    let mut tasks = self.monitor_tasks.lock().await;
                    for task in tasks.values_mut() {
                        if !task.has_expired() {
                            let _ = task.tick().await;
                        }
                    }
                }

                Ok((socket, _)) = listener.accept() => {
                    let mut self_for_loop = self_for_loop.clone();

                    let notify = self_for_loop.notify_shutdown.clone();

                    let _target_address: Option<String> = None;
                    let target = target.clone();

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
                                            match self_for_loop.handle_command(command_context.command, &target).await {
                                                Ok(Some(response)) => {
                                                    let response_data = response.serialize(&command_context.protocol);
                                                    writer.write_all(&response_data).await.unwrap();
                                                }
                                                Ok(None) => {
                                                    // Quit command - break the loop
                                                    break;
                                                }
                                                Err(e) => {
                                                    println!("Command handling error: {}", e);
                                                    let error_response = protocol::Response::Error(e.to_string());
                                                    let response_data = error_response.serialize(&command_context.protocol);
                                                    writer.write_all(&response_data).await.unwrap();
                                                }
                                            }
                                        }
                                        Err(e) if matches!(e.downcast_ref::<ParseError>(), Some(ParseError::NoCommand)) => {}
                                        Err(e) => {
                                            println!("Parse error: {}", e);
                                            // Default to text protocol for parse errors
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

        // Shutdown the writer thread and wait for jobs to complete
        if let Some(target) = target {
            if let Ok(writer) = Arc::try_unwrap(target) {
                writer.shutdown();
            }
        }

        Ok(())
    }

    async fn get_or_create_monitor_task(
        &self,
        key: &str,
        getter: &Arc<F>,
        target_writer: &Option<Arc<Writer<String>>>,
    ) -> Result<String, Error> {
        let mut tasks = self.monitor_tasks.lock().await;

        // Check if we already have a MonitorTask for this key
        if let Some(ref mut task) = tasks.get_mut(key) {
            task.touch();

            // Return the last cached value
            match task.last_result() {
                Some(ret) => return Ok(ret),
                _ => {}
            }
        }

        // Create new MonitorTask for this key
        let getter_clone = getter.clone();
        let mut monitor_task =
            MonitorTask::new(move |key: &str| -> Result<String, Error> { getter_clone(key) })
                .interval(Duration::from_secs(5))
                .key(key);

        if let Some(target_writer) = target_writer {
            monitor_task = monitor_task.target(target_writer.clone());
        }

        // Touch
        monitor_task.touch();

        // Get the current value or trigger a fresh computation
        let value = monitor_task.get().await;

        // Keep monitor_task
        tasks.insert(key.to_string(), monitor_task);
        value
    }

    async fn handle_command(
        &mut self,
        command: Command,
        target_writer: &Option<Arc<Writer<String>>>,
    ) -> anyhow::Result<Option<Response>> {
        if let Some(getter) = &self.getter {
            match command {
                Command::Get(keys) => {
                    println!("GET command with keys: {:?}", keys);
                    // Get data for each key using existing or new MonitorTask
                    let mut items = Vec::new();
                    for key in &keys {
                        match self
                            .get_or_create_monitor_task(key, getter, target_writer)
                            .await
                        {
                            Ok(value) => {
                                let item = Item {
                                    key: key.clone(),
                                    flags: 0,
                                    exptime: 0,
                                    data: value.into_bytes(),
                                    cas: None,
                                };
                                items.push(item);
                            }
                            Err(_err) => {}
                        }
                    }
                    Ok(Some(Response::Values(items)))
                }
                Command::Gets(keys) => {
                    println!("GETS command with keys: {:?}", keys);
                    let mut items = Vec::new();
                    for key in &keys {
                        match self
                            .get_or_create_monitor_task(key, getter, target_writer)
                            .await
                        {
                            Ok(value) => {
                                let item = Item {
                                    key: key.clone(),
                                    flags: 0,
                                    exptime: 0,
                                    data: value.into_bytes(),
                                    cas: Some(12345), // Include CAS for gets
                                };
                                items.push(item);
                            }
                            Err(_err) => {}
                        }
                    }
                    Ok(Some(Response::Values(items)))
                }
                Command::Gat(exptime, keys) => {
                    println!("GAT command with exptime {} and keys: {:?}", exptime, keys);
                    let mut items = Vec::new();
                    for key in &keys {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime,
                            data: b"sample_value".to_vec(),
                            cas: None,
                        };
                        items.push(item);
                    }
                    Ok(Some(Response::Values(items)))
                }
                Command::Gats(exptime, keys) => {
                    println!("GATS command with exptime {} and keys: {:?}", exptime, keys);
                    let mut items = Vec::new();
                    for key in &keys {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime,
                            data: b"sample_value".to_vec(),
                            cas: Some(12345),
                        };
                        items.push(item);
                    }
                    Ok(Some(Response::Values(items)))
                }
                Command::MetaGet(key, flags) => {
                    println!("META GET command with key: {} and flags: {:?}", key, flags);
                    // Get data for key using existing or new MonitorTask
                    if let Ok(value) = self
                        .get_or_create_monitor_task(&key, getter, target_writer)
                        .await
                    {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime: 0,
                            data: value.into_bytes(),
                            cas: Some(12345),
                        };
                        Ok(Some(Response::MetaValue(item, flags)))
                    } else {
                        Ok(Some(Response::MetaEnd))
                    }
                }
                Command::MetaNoOp => {
                    println!("META NOOP command");
                    Ok(Some(Response::MetaNoOp))
                }
                Command::Version => {
                    println!("VERSION command");
                    Ok(Some(Response::Version(self.version.clone())))
                }
                Command::Stats(arg) => {
                    println!("STATS command with arg: {:?}", arg);
                    let stats = vec![
                        ("version".to_string(), "0.1.0".to_string()),
                        ("curr_connections".to_string(), "1".to_string()),
                        ("total_connections".to_string(), "1".to_string()),
                        ("cmd_get".to_string(), "0".to_string()),
                        ("cmd_set".to_string(), "0".to_string()),
                    ];
                    Ok(Some(Response::Stats(stats)))
                }
                Command::Touch(key, exptime) => {
                    println!("TOUCH command with key: {} exptime: {}", key, exptime);
                    Ok(Some(Response::Touched))
                }
                Command::Quit => {
                    println!("QUIT command - closing connection");
                    Ok(None) // Signal that connection should close
                }
            }
        } else {
            Err(anyhow::anyhow!("No getter function configured"))
        }
    }
}

impl<F> Clone for Server<F>
where
    F: Fn(&str) -> Result<String, Error> + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            getter: self.getter.clone(),
            listen_address: self.listen_address.clone(),
            target_address: self.target_address.clone(),
            version: self.version.clone(),
            monitor_tasks: self.monitor_tasks.clone(),
            notify_shutdown: self.notify_shutdown.clone(),
        }
    }
}
