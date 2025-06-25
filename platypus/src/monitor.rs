use memcache::{Stream, ToMemcacheValue};
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct MonitorTask<V> {
    target: Option<memcache::Client>,
    interval: tokio::time::Duration,
    pub key: Option<String>,

    cancellation_token: CancellationToken,
    join_handle: Option<JoinHandle<()>>,
    last_result: Arc<RwLock<Option<V>>>,
    getter: Arc<dyn Fn(&str) -> V + Send + Sync>,
}

impl<V> MonitorTask<V>
where
    V: Display + ToMemcacheValue<Stream> + Send + Sync + Clone + 'static,
{
    pub fn new<F>(getter: F) -> MonitorTask<V>
    where
        F: Fn(&str) -> V + Send + Sync + 'static,
    {
        let cancel = CancellationToken::new();
        let last_result = Arc::new(RwLock::new(None));
        let getter = Arc::new(getter);

        MonitorTask {
            target: None,
            interval: Duration::from_secs(5),
            key: None,
            cancellation_token: cancel,
            join_handle: None,
            last_result,
            getter,
        }
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        if let Some(join_handle) = self.join_handle {
            join_handle.await
        } else {
            Ok(())
        }
    }

    pub async fn last_result(&self) -> Option<V> {
        self.last_result.read().await.clone()
    }

    pub async fn get(&self) -> Option<V> {
        if let Some(ref key) = self.key {
            let value = (self.getter)(key);
            *self.last_result.write().await = Some(value.clone());
            if let Some(ref target) = self.target {
                _ = target.set(&key, value.clone(), 60);
            }
            Some(value)
        } else {
            None
        }
    }

    pub fn target(mut self, client: memcache::Client) -> Self {
        self.target = Some(client);
        self
    }

    pub fn interval(mut self, interval: tokio::time::Duration) -> Self {
        self.interval = interval;
        self
    }

    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.into());
        self
    }

    pub async fn spawn(self) {
        let cancel_for_loop = self.cancellation_token.clone();
        let _last_result_for_loop = self.last_result.clone();
        let _getter_for_loop = self.getter.clone();

        let _future = tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            loop {
                tokio::select! {
                    _ = cancel_for_loop.cancelled() => { break }

                    _ = interval.tick() => {
                        _ = self.get().await;
                    }
                }
            }
            println!("monitor shut down");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, sleep};

    // For testing, create a version that doesn't require memcached
    pub fn spawn_test<F, R>(interval: Duration, key: String, f: F) -> MonitorTask<R>
    where
        F: Fn(&str) -> R + Send + Sync + 'static,
        R: Display + ToMemcacheValue<Stream> + Send + Sync + Clone + 'static,
    {
        let cancel = CancellationToken::new();
        let last_result = Arc::new(RwLock::new(None));
        let getter = Arc::new(f);

        let cancel_for_loop = cancel.clone();
        let last_result_for_loop = last_result.clone();
        let getter_for_loop = getter.clone();
        let key_for_loop = key.clone();

        let future = tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = cancel_for_loop.cancelled() => { break }

                    _ = interval.tick() => {
                        let ret = getter_for_loop(&key_for_loop);
                        *last_result_for_loop.write().await = Some(ret.clone());
                        // Skip memcached for tests
                    }
                }
            }
        });

        MonitorTask {
            target: None,
            interval,
            key: Some(key),
            cancellation_token: cancel,
            join_handle: Some(future),
            last_result,
            getter,
        }
    }

    #[tokio::test]
    async fn test_monitor_task_last_value() {
        let task = spawn_test(Duration::from_millis(100), "test_key".to_string(), |_| "test_value".to_string());

        // Initially, no value should be available
        assert!(task.last_result().await.is_none());

        // Wait a bit for the first interval to trigger
        sleep(Duration::from_millis(150)).await;

        // Now there should be a value
        assert_eq!(task.last_result().await, Some("test_value".to_string()));

        task.cancel();
    }

    #[tokio::test]
    async fn test_monitor_task_get() {
        let task = spawn_test(Duration::from_millis(1000), "test_key".to_string(), |_| "computed_value".to_string());

        // Get should compute and return the value immediately
        let value = task.get().await;
        assert_eq!(value, Some("computed_value".to_string()));

        // last_result should now return the computed value
        assert_eq!(task.last_result().await, Some("computed_value".to_string()));

        task.cancel();
    }

    #[tokio::test]
    async fn test_monitor_task_cancel() {
        let task = spawn_test(Duration::from_millis(100), "test_key".to_string(), |_| "test_value".to_string());

        assert!(!task.is_cancelled());
        task.cancel();
        assert!(task.is_cancelled());
    }
}
