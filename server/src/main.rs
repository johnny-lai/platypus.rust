use anyhow::Result;
use futures::future::join_all;
use platypus::Monitor;
use platypus::protocol::{self, Command, Response, Item};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, tcp::{OwnedWriteHalf}};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

async fn handle_command(
    command: Command,
    writer: &mut OwnedWriteHalf,
    monitors: &Arc<Mutex<Vec<CancellationToken>>>,
    handles: &Arc<Mutex<Vec<JoinHandle<()>>>>,
) -> bool {
    match command {
        Command::Get(keys) => {
            println!("GET command with keys: {:?}", keys);
            // Simulate getting data for each key
            for key in &keys {
                let s = Monitor::new(Duration::from_secs(2), key.clone());
                let (cancellation, join) = s.spawn(|| "stuff");
                monitors.lock().await.push(cancellation);
                handles.lock().await.push(join);
                
                // Create a sample item
                let item = Item {
                    key: key.clone(),
                    flags: 0,
                    exptime: 0,
                    data: b"sample_value".to_vec(),
                    cas: None,
                };
                let response = Response::Value(item);
                writer.write_all(response.format().as_bytes()).await.unwrap();
            }
            writer.write_all(b"END\r\n").await.unwrap();
        },
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
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
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
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
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
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
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
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
        Command::MetaNoOp => {
            println!("META NOOP command");
            let response = Response::MetaNoOp;
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
        Command::Version => {
            println!("VERSION command");
            let response = Response::Version("0.1.0".to_string());
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
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
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
        Command::Touch(key, exptime) => {
            println!("TOUCH command with key: {} exptime: {}", key, exptime);
            let response = Response::Touched;
            writer.write_all(response.format().as_bytes()).await.unwrap();
        },
        Command::Quit => {
            println!("QUIT command - closing connection");
            return true; // Signal that connection should close
        },
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

    let handles = Arc::new(Mutex::new(Vec::new()));
    let monitors = Arc::new(Mutex::new(Vec::new()));
    loop {
        tokio::select! {
            _ = notify_shutdown.notified() => {
                break;
            }

            Ok((socket, _)) = listener.accept() => {
                let notify = notify_shutdown.clone();
                let handles = handles.clone();
                let monitors = monitors.clone();

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
                                        let should_close = handle_command(command, &mut writer, &monitors, &handles).await;
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
    for c in monitors.lock().await.iter() {
        c.cancel();
    }
    let handles = handles.lock().await.drain(..).collect::<Vec<_>>();
    join_all(handles).await;

    Ok(())
}
