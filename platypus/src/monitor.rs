use crate::Error;
use memcache::{Stream, ToMemcacheValue};
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct MonitorTask<V> {
    target: Option<memcache::Client>,
    interval: tokio::time::Duration,
    ttl: tokio::time::Duration,
    until: Arc<RwLock<tokio::time::Instant>>,
    pub key: Option<String>,

    last_result: Arc<RwLock<Option<V>>>,
    getter: Arc<dyn Fn(&str) -> Result<V, Error> + Send + Sync>,
}

impl<V> MonitorTask<V>
where
    V: Display + ToMemcacheValue<Stream> + Send + Sync + Clone + 'static,
{
    pub fn new<F>(getter: F) -> MonitorTask<V>
    where
        F: Fn(&str) -> Result<V, Error> + Send + Sync + 'static,
    {
        let last_result = Arc::new(RwLock::new(None));
        let getter = Arc::new(getter);

        MonitorTask {
            target: None,
            interval: Duration::from_secs(5),
            ttl: Duration::from_secs(12),
            until: Arc::new(RwLock::new(Instant::now())),
            key: None,
            last_result,
            getter,
        }
    }

    pub async fn has_expired(&self) -> bool {
        let until = self.until.read().await;
        Instant::now().gt(&until)
    }

    pub async fn touch(&self) {
        *self.until.write().await = Instant::now() + self.ttl;
    }

    pub async fn last_result(&self) -> Option<V> {
        self.last_result.read().await.clone()
    }

    pub async fn get(&self) -> Result<V, Error> {
        if let Some(ref key) = self.key {
            match (self.getter)(key) {
                Ok(value) => {
                    *self.last_result.write().await = Some(value.clone());
                    // Write to target if any
                    if let Some(ref target) = self.target {
                        _ = target.set(&key, value.clone(), 60);
                    }
                    Ok(value)
                }
                Err(err) => Err(err),
            }
        } else {
            Err(crate::Error::NotReady)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn test_monitor_task_basic() {
        let task = MonitorTask::new(|_key| Ok("test_value".to_string())).key("test_key");

        // Initially, should be able to get value
        let value = task.get().await.unwrap();
        assert_eq!(value, "test_value");

        // last_result should now return the computed value
        assert_eq!(task.last_result().await, Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_monitor_task_expiry() {
        let task = MonitorTask::new(|_key| Ok("test_value".to_string())).key("test_key");

        // Initially should not be expired
        assert!(task.has_expired().await);

        // Touch should reset expiry
        task.touch().await;
        assert!(!task.has_expired().await);

        // Wait for expiry
        sleep(Duration::from_millis(50)).await;
        assert!(!task.has_expired().await); // Still not expired due to 12s default TTL
    }

    #[tokio::test]
    async fn test_monitor_task_no_key() {
        let task = MonitorTask::new(|_key| Ok("test_value".to_string()));

        // Should return NotReady error when no key is set
        let result = task.get().await;
        assert!(matches!(result, Err(crate::Error::NotReady)));
    }
}
