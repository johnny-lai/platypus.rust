use anyhow::Result;
use futures::future::join_all;
use procyon::Monitor;
use procyon::protocol::{self, Command};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

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

                                match protocol::text::parse(&line) {
                                    Ok(Command::Get(key)) => {
                                        let s = Monitor::new(Duration::from_secs(2), key.clone());
                                        let (cancellation, join) = s.spawn(|| {
                                            "stuff"
                                        });
                                        monitors.lock().await.push(cancellation);
                                        handles.lock().await.push(join);

                                        println!("command get got {}", key);
                                        let value = "response";
                                        writer
                                            .write_all(
                                                format!("VALUE {} 0 {}\r\n{}\r\nEND\r\n", key, value.len(), value)
                                                    .as_bytes(),
                                            )
                                            .await
                                            .unwrap();
                                    }
                                    Ok(Command::Version) => {
                                        println!("version");
                                        writer.write_all(b"VERSION 0.0.1\r\n").await.unwrap();
                                    }
                                    Err(error) => {
                                        println!("unknown command {}", error);
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
