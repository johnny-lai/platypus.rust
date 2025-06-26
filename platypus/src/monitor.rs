use crate::Error;
use memcache::{Stream, ToMemcacheValue};
use std::fmt::Display;
use std::sync::Arc;
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct MonitorTask<V> {
    target: Option<memcache::Client>,
    interval: tokio::time::Duration,
    ttl: tokio::time::Duration,
    until: tokio::time::Instant,
    updated_at: tokio::time::Instant,
    pub key: Option<String>,

    last_result: Option<V>,
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
        let getter = Arc::new(getter);

        MonitorTask {
            target: None,
            interval: Duration::from_secs(5),
            ttl: Duration::from_secs(12),
            until: Instant::now(),
            updated_at: Instant::now(),
            key: None,
            last_result: None,
            getter,
        }
    }

    pub fn has_expired(&self) -> bool {
        let until = self.until;
        Instant::now().gt(&until)
    }

    pub fn touch(&mut self) {
        self.until = Instant::now() + self.ttl;
    }

    pub fn last_result(&self) -> Option<V> {
        self.last_result.clone()
    }

    pub async fn get(&mut self) -> Result<V, Error> {
        if let Some(ref key) = self.key {
            match (self.getter)(key) {
                Ok(value) => {
                    self.last_result = Some(value.clone());
                    // Write to target if any
                    if let Some(ref target) = self.target {
                        _ = target.set(&key, value.clone(), self.ttl.as_secs() as u32);
                    }
                    Ok(value)
                }
                Err(err) => Err(err),
            }
        } else {
            Err(crate::Error::NotReady)
        }
    }

    pub async fn tick(&mut self) -> bool {
        if self.has_expired() {
            return false;
        }

        if Instant::now().gt(&(self.updated_at + self.interval)) {
            let _ = self.get().await;
        }

        true
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
        let mut task = MonitorTask::new(|_key| Ok("test_value".to_string())).key("test_key");

        // Initially, should be able to get value
        let value = task.get().await.unwrap();
        assert_eq!(value, "test_value");

        // last_result should now return the computed value
        assert_eq!(task.last_result(), Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_monitor_task_expiry() {
        let mut task = MonitorTask::new(|_key| Ok("test_value".to_string())).key("test_key");

        // Initially should not be expired
        assert!(task.has_expired());

        // Touch should reset expiry
        task.touch();
        assert!(!task.has_expired());

        // Wait for expiry
        sleep(Duration::from_millis(50));
        assert!(!task.has_expired()); // Still not expired due to 12s default TTL
    }

    #[tokio::test]
    async fn test_monitor_task_no_key() {
        let mut task = MonitorTask::new(|_key| Ok("test_value".to_string()));

        // Should return NotReady error when no key is set
        let result = task.get().await;
        assert!(matches!(result, Err(crate::Error::NotReady)));
    }
}
