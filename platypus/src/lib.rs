use memcache::{Stream, ToMemcacheValue};
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub mod protocol;

pub struct Monitor {
    interval: tokio::time::Duration,
    key: String,
}

pub struct MonitorTask<R> {
    cancellation_token: CancellationToken,
    join_handle: JoinHandle<()>,
    last_value: Arc<RwLock<Option<R>>>,
    closure: Arc<dyn Fn() -> R + Send + Sync + 'static>,
}

impl<R> MonitorTask<R> 
where
    R: Clone + Send + Sync + 'static,
{
    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        self.join_handle.await
    }

    pub async fn last_value(&self) -> Option<R> {
        self.last_value.read().await.clone()
    }

    pub async fn get(&self) -> R {
        let result = (self.closure)();
        *self.last_value.write().await = Some(result.clone());
        result
    }
}

impl Monitor {
    pub fn new(interval: tokio::time::Duration, key: String) -> Self {
        Self { interval, key }
    }

    pub fn spawn<F, R>(self, f: F) -> MonitorTask<R>
    where
        F: Fn() -> R + Send + Sync + 'static,
        R: Display + ToMemcacheValue<Stream> + Send + Sync + Clone + 'static,
    {
        let cancel = CancellationToken::new();
        let backend = memcache::Client::connect("memcache://127.0.0.1:11213").unwrap();
        let last_value = Arc::new(RwLock::new(None));
        let closure = Arc::new(f);

        let cancel_for_loop = cancel.clone();
        let last_value_for_loop = last_value.clone();
        let closure_for_loop = closure.clone();
        let key = self.key.clone();
        
        let future = tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            loop {
                tokio::select! {
                    _ = cancel_for_loop.cancelled() => { break }

                    _ = interval.tick() => {
                        let ret = closure_for_loop();
                        *last_value_for_loop.write().await = Some(ret.clone());
                        _ = backend.set(key.as_str(), ret, 60);
                    }
                }
            }
            println!("monitor shut down");
        });

        MonitorTask {
            cancellation_token: cancel,
            join_handle: future,
            last_value,
            closure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    // For testing, create a version that doesn't require memcached
    pub fn spawn_test<F, R>(monitor: Monitor, f: F) -> MonitorTask<R>
    where
        F: Fn() -> R + Send + Sync + 'static,
        R: Send + Sync + Clone + 'static,
    {
        let cancel = CancellationToken::new();
        let last_value = Arc::new(RwLock::new(None));
        let closure = Arc::new(f);

        let cancel_for_loop = cancel.clone();
        let last_value_for_loop = last_value.clone();
        let closure_for_loop = closure.clone();
        
        let future = tokio::spawn(async move {
            let mut interval = tokio::time::interval(monitor.interval);
            loop {
                tokio::select! {
                    _ = cancel_for_loop.cancelled() => { break }

                    _ = interval.tick() => {
                        let ret = closure_for_loop();
                        *last_value_for_loop.write().await = Some(ret.clone());
                        // Skip memcached for tests
                    }
                }
            }
        });

        MonitorTask {
            cancellation_token: cancel,
            join_handle: future,
            last_value,
            closure,
        }
    }

    #[tokio::test]
    async fn test_monitor_task_last_value() {
        let monitor = Monitor::new(Duration::from_millis(100), "test_key".to_string());
        let task = spawn_test(monitor, || "test_value".to_string());
        
        // Initially, no value should be available
        assert!(task.last_value().await.is_none());
        
        // Wait a bit for the first interval to trigger
        sleep(Duration::from_millis(150)).await;
        
        // Now there should be a value
        assert_eq!(task.last_value().await, Some("test_value".to_string()));
        
        task.cancel();
    }

    #[tokio::test]
    async fn test_monitor_task_get() {
        let monitor = Monitor::new(Duration::from_millis(1000), "test_key".to_string());
        let task = spawn_test(monitor, || "computed_value".to_string());
        
        // Get should compute and return the value immediately
        let value = task.get().await;
        assert_eq!(value, "computed_value".to_string());
        
        // last_value should now return the computed value
        assert_eq!(task.last_value().await, Some("computed_value".to_string()));
        
        task.cancel();
    }

    #[tokio::test]
    async fn test_monitor_task_cancel() {
        let monitor = Monitor::new(Duration::from_millis(100), "test_key".to_string());
        let task = spawn_test(monitor, || "test_value".to_string());
        
        assert!(!task.is_cancelled());
        task.cancel();
        assert!(task.is_cancelled());
    }
}
