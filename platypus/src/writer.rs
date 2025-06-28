use memcache::{Stream, ToMemcacheValue};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use tokio::time::Duration;

#[derive(Clone)]
pub struct WriteJob<V> {
    key: String,
    value: V,
    ttl_secs: u32,
}

pub struct Writer<V> {
    sender: Sender<WriteJob<V>>,
    shutdown_sender: Sender<()>,
    handle: JoinHandle<()>,
}

impl<V> Writer<V>
where
    V: ToMemcacheValue<Stream> + Send + Sync + 'static,
{
    pub fn new(target_address: &str) -> Self {
        let (tx, rx) = channel::<WriteJob<V>>();
        let (shutdown_tx, shutdown_rx) = channel::<()>();

        let client = memcache::Client::connect(target_address).unwrap();

        let handle = std::thread::Builder::new()
            .name(format!("writer/{}", target_address))
            .spawn(move || {
                loop {
                    match rx.try_recv() {
                        Ok(job) => {
                            _ = client.set(job.key.as_str(), job.value, job.ttl_secs);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            // Check for shutdown signal
                            if shutdown_rx.try_recv().is_ok() {
                                // Process remaining jobs before shutting down
                                while let Ok(job) = rx.try_recv() {
                                    _ = client.set(job.key.as_str(), job.value, job.ttl_secs);
                                }
                                break;
                            }
                            // Brief sleep to avoid busy waiting
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            // Process remaining jobs before shutting down
                            while let Ok(job) = rx.try_recv() {
                                _ = client.set(job.key.as_str(), job.value, job.ttl_secs);
                            }
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn memcache writer thread");

        Writer {
            sender: tx,
            shutdown_sender: shutdown_tx,
            handle,
        }
    }

    pub fn send(
        &self,
        key: &str,
        value: V,
        ttl: Duration,
    ) -> Result<(), std::sync::mpsc::SendError<WriteJob<V>>> {
        let ttl_secs = ttl.as_secs() as u32;
        let job = WriteJob {
            key: key.into(),
            value,
            ttl_secs,
        };
        self.sender.send(job)
    }

    pub fn shutdown(self) {
        // Signal shutdown
        let _ = self.shutdown_sender.send(());
        // Wait for thread to finish processing all jobs
        let _ = self.handle.join();
    }
}
