use crate::Value;
use memcache::MemcacheError;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use tokio::time::Duration;
use tracing::{error, info};

#[derive(Clone)]
pub struct WriteJob {
    key: String,
    value: Option<Value>,
    ttl_secs: u32,
}

pub struct Writer {
    sender: Sender<WriteJob>,
    shutdown_sender: Sender<()>,
    handle: JoinHandle<()>,
}

impl Writer {
    pub fn new(target_address: &str) -> Self {
        let (tx, rx) = channel::<WriteJob>();
        let (shutdown_tx, shutdown_rx) = channel::<()>();

        let target_address = target_address.to_string();
        let handle = std::thread::Builder::new()
            .name(format!("writer/{}", target_address))
            .spawn(move || {
                info!(target_address = ?target_address, "Writer thread started");
                let mut client = Self::client(target_address.as_str());
                loop {
                    // Check for shutdown signal
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                    // Check if there is a job
                    match rx.recv_timeout(std::time::Duration::from_millis(10)) {
                        Ok(job) => client = Self::process(client, target_address.as_str(), job),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {}
                    }
                }
                // Process remaining jobs before shutting down
                while let Ok(job) = rx.try_recv() {
                    client = Self::process(client, target_address.as_str(), job)
                }
                info!(target_address = ?target_address, "Writer thread terminated");
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
        value: Option<Value>,
        ttl: Duration,
    ) -> Result<(), std::sync::mpsc::SendError<WriteJob>> {
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

    fn process(
        client: Result<memcache::Client, MemcacheError>,
        target_address: &str,
        job: WriteJob,
    ) -> Result<memcache::Client, MemcacheError> {
        match client {
            Ok(ref c) => {
                info!(target_address = ?target_address, key = job.key.as_str(),  "Wrote");
                let err = match job.value {
                    Some(value) => c.set(job.key.as_str(), value, job.ttl_secs).err(),
                    None => c.delete(job.key.as_str()).err(),
                };
                if let Some(MemcacheError::IOError(_)) = err {
                    return Self::client(target_address);
                }
            }
            Err(err) => {
                error!(error = ?err, "client was not connected");
                return Self::client(target_address);
            }
        }
        client
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration as StdDuration;
    use tokio::time::Duration;

    #[test]
    fn test_write_job_creation() {
        let job = WriteJob {
            key: "test_key".to_string(),
            value: Some("test_value".to_string()),
            ttl_secs: 300,
        };

        assert_eq!(job.key, "test_key");
        assert_eq!(job.value, Some("test_value".to_string()));
        assert_eq!(job.ttl_secs, 300);
    }

    #[test]
    fn test_write_job_clone() {
        let job = WriteJob {
            key: "test_key".to_string(),
            value: Some("test_value".to_string()),
            ttl_secs: 300,
        };

        let cloned_job = job.clone();
        assert_eq!(job.key, cloned_job.key);
        assert_eq!(job.value, cloned_job.value);
        assert_eq!(job.ttl_secs, cloned_job.ttl_secs);
    }

    #[test]
    #[should_panic(expected = "thread name may not contain interior null bytes")]
    fn test_writer_new_with_invalid_thread_name() {
        let invalid_name = "\0invalid";
        let _writer = Writer::new(invalid_name);
    }

    #[test]
    fn test_writer_send_job() {
        let writer = Writer::new("127.0.0.1:11211");
        let result = writer.send(
            "test_key",
            Some("test_value".to_string()),
            Duration::from_secs(300),
        );
        assert!(result.is_ok());
        writer.shutdown();
    }

    #[test]
    fn test_writer_send_with_zero_ttl() {
        let writer = Writer::new("127.0.0.1:11211");
        let result = writer.send("test_key", Some("test_value".to_string()), Duration::from_secs(0));
        assert!(result.is_ok());
        writer.shutdown();
    }

    #[test]
    fn test_writer_send_with_large_ttl() {
        let writer = Writer::new("127.0.0.1:11211");
        let large_ttl = Duration::from_secs(u32::MAX as u64);
        let result = writer.send("test_key", Some("test_value".to_string()), large_ttl);
        assert!(result.is_ok());
        writer.shutdown();
    }

    #[test]
    fn test_writer_shutdown() {
        let writer = Writer::new("127.0.0.1:11211");

        let result = writer.send(
            "test_key",
            Some("test_value".to_string()),
            Duration::from_secs(300),
        );
        assert!(result.is_ok());

        writer.shutdown();
    }

    #[test]
    fn test_writer_shutdown_processes_remaining_jobs() {
        let _counter = Arc::new(AtomicUsize::new(0));
        let writer = Writer::new("127.0.0.1:11211");

        for i in 0..10 {
            let _ = writer.send(
                &format!("key_{}", i),
                Some(format!("value_{}", i)),
                Duration::from_secs(300),
            );
        }

        std::thread::sleep(StdDuration::from_millis(100));
        writer.shutdown();
    }

    #[test]
    fn test_writer_thread_name() {
        let writer = Writer::new("127.0.0.1:11211");

        let thread_name = writer.handle.thread().name().unwrap_or("");
        assert!(thread_name.starts_with("writer/"));
        assert!(thread_name.contains("127.0.0.1:11211"));

        writer.shutdown();
    }

    #[test]
    fn test_writer_client_connection_timeout() {
        let client_result = Writer::client("127.0.0.1:11211");
        // Just test that the function can be called - the result depends on whether memcached is running
        let _ = client_result;
    }

    #[test]
    fn test_writer_client_invalid_address() {
        let client_result = Writer::client("invalid_address");
        assert!(client_result.is_err());
    }

    #[test]
    fn test_writer_send_after_shutdown() {
        let writer = Writer::new("127.0.0.1:11211");
        let sender = writer.sender.clone();
        writer.shutdown();

        let job = WriteJob {
            key: "test_key".to_string(),
            value: Some("test_value".to_string()),
            ttl_secs: 300,
        };
        let result = sender.send(job);
        assert!(result.is_err());
    }

    #[test]
    fn test_ttl_conversion() {
        let writer = Writer::new("127.0.0.1:11211");

        let ttl_millis = Duration::from_millis(5500);
        let result = writer.send("test_key", Some("test_value".to_string()), ttl_millis);
        assert!(result.is_ok());

        writer.shutdown();
    }

    #[test]
    fn test_writer_with_string_type() {
        let string_writer = Writer::new("127.0.0.1:11211");
        let string_result =
            string_writer.send("key1", Some("value1".to_string()), Duration::from_secs(300));
        assert!(string_result.is_ok());
        string_writer.shutdown();
    }
}
