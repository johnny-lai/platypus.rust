use anyhow::Result;
use futures::future::join_all;
use platypus::{Monitor, MonitorTask};
use platypus::protocol::{self, Command, Item, Response};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, tcp::OwnedWriteHalf};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, Notify};
use tokio::time::Duration;

async fn handle_command(
    command: Command,
    writer: &mut OwnedWriteHalf,
    monitor_tasks: &Arc<Mutex<Vec<MonitorTask<String>>>>,
) -> bool {
    match command {
        Command::Get(keys) => {
            println!("GET command with keys: {:?}", keys);
            // Simulate getting data for each key
            for key in &keys {
                let s = Monitor::new(Duration::from_secs(2), key.clone());
                let monitor_task = s.spawn(|| "stuff".to_string());
                
                // Get the current value or trigger a fresh computation
                let value = if let Some(cached_value) = monitor_task.last_value().await {
                    cached_value
                } else {
                    monitor_task.get().await
                };
                
                monitor_tasks.lock().await.push(monitor_task);

                // Create a sample item
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
            writer.write_all(b"END\r\n").await.unwrap();
        }
        Command::Gets(keys) => {
            println!("GETS command with keys: {:?}", keys);
            let mut items = Vec::new();
            for key in &keys {
                let item = Item {
                    key: key.clone(),
                    flags: 0,
                    exptime: 0,
                    data: b"sample_value".to_vec(),
                    cas: Some(12345), // Include CAS for gets
                };
                items.push(item);
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
            let item = Item {
                key: key.clone(),
                flags: 0,
                exptime: 0,
                data: b"sample_value".to_vec(),
                cas: Some(12345),
            };
            let response = Response::MetaValue(item, flags);
            writer
                .write_all(response.format().as_bytes())
                .await
                .unwrap();
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:11212").await?;
    let notify_shutdown = Arc::new(Notify::new());

    // Trigger shutdown on Ctrl+C
    let notify_shutdown_on_ctrl_c = notify_shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("Shutting down");
        notify_shutdown_on_ctrl_c.notify_waiters();
    });

    // Trigger shutdown on Linux TERM signal
    let notify_shutdown_on_term = notify_shutdown.clone();
    tokio::spawn(async move {
        if let Ok(mut term_signal) = signal(SignalKind::terminate()) {
            term_signal.recv().await.unwrap();
            println!("Shutting down");
            notify_shutdown_on_term.notify_waiters();
        }
    });

    let monitor_tasks: Arc<Mutex<Vec<MonitorTask<String>>>> = Arc::new(Mutex::new(Vec::new()));
    loop {
        tokio::select! {
            _ = notify_shutdown.notified() => {
                break;
            }

            Ok((socket, _)) = listener.accept() => {
                let notify = notify_shutdown.clone();
                let monitor_tasks = monitor_tasks.clone();

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
                                        let should_close = handle_command(command, &mut writer, &monitor_tasks).await;
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
    let tasks = monitor_tasks.lock().await.drain(..).collect::<Vec<_>>();
    for task in &tasks {
        task.cancel();
    }
    // Wait for all monitor tasks to complete
    let join_handles: Vec<_> = tasks.into_iter().map(|task| async move { task.join().await }).collect();
    join_all(join_handles).await;

    Ok(())
}
