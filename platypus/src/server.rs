use crate::monitor::MonitorTask;
use crate::protocol::{self, Command, Item, Response};
use anyhow::Result;
use futures::future::join_all;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, tcp::OwnedWriteHalf};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, Notify};
use tokio::time::Duration;

#[derive(Default)]
pub struct Server<F> {
    listen_address: String,
    target_address: Option<String>,
    getter: Option<Arc<F>>,
    monitor_tasks: Arc<Mutex<HashMap<String, MonitorTask<String>>>>,
    notify_shutdown: Arc<Notify>,
}

impl<F> Server<F>
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    pub fn bind(listen_address: &str) -> Self {
        Self {
            listen_address: listen_address.to_owned(),
            target_address: None,
            getter: None,
            monitor_tasks: Arc::new(Mutex::new(HashMap::new())),
            notify_shutdown: Arc::new(Notify::new()),
        }
    }

    pub fn getter(mut self, f: F) -> Self {
        self.getter = Some(Arc::new(f));
        self
    }

    pub fn target(mut self, target_address: &str) -> Self {
        self.target_address = Some(target_address.to_owned());
        self
    }

    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::bind(self.listen_address).await?;

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

        loop {
            tokio::select! {
                _ = self.notify_shutdown.notified() => {
                    break;
                }

                Ok((socket, _)) = listener.accept() => {
                    let notify = self.notify_shutdown.clone();
                    let monitor_tasks = self.monitor_tasks.clone();
                    let getter = self.getter.clone();
                    let target_address = self.target_address.clone();

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

                                bytes = reader.read_line(&mut line) => {
                                    if bytes.unwrap_or(0) == 0 {
                                        break;
                                    }

                                    match protocol::parse(&line) {
                                        Ok(command) => {
                                            let should_close = Self::handle_command_static(command, &mut writer, &monitor_tasks, &target_address, &getter).await;
                                            if should_close {
                                                break;
                                            }
                                        }
                                        Err(error) => {
                                            println!("Parse error: {}", error);
                                            writer.write_all(b"ERROR\r\n").await.unwrap();
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

        // Shutdown
        let tasks = self
            .monitor_tasks
            .lock()
            .await
            .drain()
            .map(|(_, task)| task)
            .collect::<Vec<_>>();
        for task in &tasks {
            task.cancel();
        }
        // Wait for all monitor tasks to complete
        let join_handles: Vec<_> = tasks
            .into_iter()
            .map(|task| async move { task.join().await })
            .collect();
        join_all(join_handles).await;

        Ok(())
    }

    async fn get_or_create_monitor_task(
        key: &str,
        getter: &Arc<F>,
        target_address: &Option<String>,
        monitor_tasks: &Arc<Mutex<HashMap<String, MonitorTask<String>>>>,
    ) -> Option<String> {
        let mut tasks = monitor_tasks.lock().await;

        // Check if we already have a MonitorTask for this key
        if let Some(task) = tasks.get(key) {
            // Return the last cached value
            return task.last_result().await;
        }

        // Create new MonitorTask for this key
        let getter_clone = getter.clone();
        let mut monitor_task = MonitorTask::new(move |key: &str| -> String { getter_clone(key) })
            .interval(Duration::from_secs(5))
            .key(key);

        if let Some(target_address) = target_address {
            let client = memcache::Client::connect(target_address.as_str()).unwrap();
            monitor_task = monitor_task.target(client);
        }

        // Get the current value or trigger a fresh computation
        let value = monitor_task.get().await;
        tasks.insert(key.to_string(), monitor_task);
        value
    }

    async fn handle_command_static(
        command: Command,
        writer: &mut OwnedWriteHalf,
        monitor_tasks: &Arc<Mutex<HashMap<String, MonitorTask<String>>>>,
        target_address: &Option<String>,
        getter: &Option<Arc<F>>,
    ) -> bool {
        if let Some(getter) = getter {
            match command {
                Command::Get(keys) => {
                    println!("GET command with keys: {:?}", keys);
                    // Get data for each key using existing or new MonitorTask
                    for key in &keys {
                        if let Some(value) = Self::get_or_create_monitor_task(
                            key,
                            getter,
                            target_address,
                            monitor_tasks,
                        )
                        .await
                        {
                            let item = Item {
                                key: key.clone(),
                                flags: 0,
                                exptime: 0,
                                data: value.into_bytes(),
                                cas: None,
                            };
                            let response = Response::Value(item);
                            writer
                                .write_all(response.format().as_bytes())
                                .await
                                .unwrap();
                        }
                    }
                    writer.write_all(b"END\r\n").await.unwrap();
                }
                Command::Gets(keys) => {
                    println!("GETS command with keys: {:?}", keys);
                    let mut items = Vec::new();
                    for key in &keys {
                        if let Some(value) = Self::get_or_create_monitor_task(
                            key,
                            getter,
                            target_address,
                            monitor_tasks,
                        )
                        .await
                        {
                            let item = Item {
                                key: key.clone(),
                                flags: 0,
                                exptime: 0,
                                data: value.into_bytes(),
                                cas: Some(12345), // Include CAS for gets
                            };
                            items.push(item);
                        }
                    }
                    let response = Response::Values(items);
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
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
                    let response = Response::Values(items);
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
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
                    let response = Response::Values(items);
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
                }
                Command::MetaGet(key, flags) => {
                    println!("META GET command with key: {} and flags: {:?}", key, flags);
                    // Get data for key using existing or new MonitorTask
                    if let Some(value) = Self::get_or_create_monitor_task(
                        &key,
                        getter,
                        target_address,
                        monitor_tasks,
                    )
                    .await
                    {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime: 0,
                            data: value.into_bytes(),
                            cas: Some(12345),
                        };
                        let response = Response::MetaValue(item, flags);
                        writer
                            .write_all(response.format().as_bytes())
                            .await
                            .unwrap();
                    }

                    writer.write_all(b"END\r\n").await.unwrap();
                }
                Command::MetaNoOp => {
                    println!("META NOOP command");
                    let response = Response::MetaNoOp;
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
                }
                Command::Version => {
                    println!("VERSION command");
                    let response = Response::Version("0.1.0".to_string());
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
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
                    let response = Response::Stats(stats);
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
                }
                Command::Touch(key, exptime) => {
                    println!("TOUCH command with key: {} exptime: {}", key, exptime);
                    let response = Response::Touched;
                    writer
                        .write_all(response.format().as_bytes())
                        .await
                        .unwrap();
                }
                Command::Quit => {
                    println!("QUIT command - closing connection");
                    return true; // Signal that connection should close
                }
            }
            false // Continue processing commands
        } else {
            true // Signal that connection should close
        }
    }
}
