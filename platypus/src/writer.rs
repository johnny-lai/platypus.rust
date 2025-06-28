use memcache::{MemcacheError, Stream, ToMemcacheValue};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
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

        let target_address = target_address.to_string();
        let handle = std::thread::Builder::new()
            .name(format!("writer/{}", target_address))
            .spawn(move || {
                let mut client = Self::client(target_address.as_str());
                loop {
                    // Check for shutdown signal
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                    // Check if there is a job
                    match rx.recv_timeout(std::time::Duration::from_millis(10)) {
                        Ok(job) => {
                            if let Ok(ref c) = client {
                                match c.set(job.key.as_str(), job.value, job.ttl_secs) {
                                    Ok(_) => {}
                                    Err(MemcacheError::IOError(_)) => {
                                        client = Self::client(target_address.as_str());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {}
                    }
                }
                // Process remaining jobs before shutting down
                while let Ok(job) = rx.try_recv() {
                    if let Ok(ref c) = client {
                        _ = c.set(job.key.as_str(), job.value, job.ttl_secs);
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

    fn client(target_address: &str) -> Result<memcache::Client, MemcacheError> {
        memcache::Client::builder()
            .with_connection_timeout(std::time::Duration::from_secs(1))
            .add_server(target_address)?
            .build()
    }
}
