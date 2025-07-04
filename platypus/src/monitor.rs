use crate::writer::Writer;
use memcache::{Stream, ToMemcacheValue};
use std::collections::HashMap;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use tracing::debug;

pub struct MonitorTask<V> {
    // Period at getter will be called to refresh the value
    interval: Duration,

    // Duration key on the target should be kept for
    // This should be greater than interval.
    // Every usage of getter will keep the refresh monitor alive for this period of time
    ttl: Duration,

    // Refresh will keep running until this instant
    until: Instant,

    // Last time refresh occurred
    updated_at: Instant,

    // The key the value will be written to in the target
    pub key: Option<String>,

    // Last result getter returned
    last_result: Option<V>,

    // Result
    getter: Arc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = Option<V>> + Send + '_>> + Send + Sync>,

    // The target where updated values will be written to
    target: Option<Arc<Writer<V>>>,
}

impl<V> MonitorTask<V>
where
    V: Display + ToMemcacheValue<Stream> + Send + Sync + Clone + 'static,
{
    pub fn new<F>(getter: F) -> MonitorTask<V>
    where
        F: Fn(&str) -> Pin<Box<dyn Future<Output = Option<V>> + Send + '_>> + Send + Sync + 'static,
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

    pub async fn get(&mut self) -> Option<V> {
        if let Some(ref key) = self.key {
            match (self.getter)(key).await {
                Some(value) => {
                    self.updated_at = Instant::now();
                    self.last_result = Some(value.clone());
                    if let Some(target) = &self.target {
                        let value = value.clone();
                        target.send(key, value, self.ttl).ok();
                    }
                    Some(value)
                }
                None => None,
            }
        } else {
            None
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

    pub fn target(mut self, target: Arc<Writer<V>>) -> Self {
        self.target = Some(target);
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

#[derive(Clone)]
pub struct MonitorTasks<V> {
    tasks: Arc<Mutex<HashMap<String, MonitorTask<V>>>>,
}

impl<V> MonitorTasks<V>
where
    V: Display + ToMemcacheValue<Stream> + Send + Sync + Clone + 'static,
{
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_or_create_task<F>(
        &self,
        key: &str,
        getter: &Arc<F>,
        target_writer: &Option<Arc<Writer<V>>>,
    ) -> Option<V>
    where
        F: Fn(&str) -> Pin<Box<dyn Future<Output = Option<V>> + Send + '_>> + Send + Sync + 'static,
    {
        let mut tasks = self.tasks.lock().await;

        if let Some(ref mut task) = tasks.get_mut(key) {
            task.touch();
            match task.last_result() {
                Some(ret) => return Some(ret),
                _ => {}
            }
        }

        debug!(key= ?key, "New MonitorTask");
        let getter_clone = getter.clone();
        let mut monitor_task = MonitorTask::new(move |key: &str| getter_clone(key))
            .interval(Duration::from_secs(5))
            .key(key);

        if let Some(target_writer) = target_writer {
            monitor_task = monitor_task.target(target_writer.clone());
        }

        monitor_task.touch();
        let value = monitor_task.get().await;
        tasks.insert(key.to_string(), monitor_task);
        value
    }

    pub async fn tick(&self) {
        let mut tasks = self.tasks.lock().await;
        for task in tasks.values_mut() {
            if !task.has_expired() {
                _ = task.tick().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn test_monitor_task_basic() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .key("test_key");

        // Initially, should be able to get value
        let value = task.get().await.unwrap();
        assert_eq!(value, "test_value");

        // last_result should now return the computed value
        assert_eq!(task.last_result(), Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_monitor_task_expiry() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .key("test_key");

        // Initially should not be expired
        assert!(task.has_expired());

        // Touch should reset expiry
        task.touch();
        assert!(!task.has_expired());

        // Wait for expiry
        sleep(Duration::from_millis(50)).await;
        assert!(!task.has_expired()); // Still not expired due to 12s default TTL
    }

    #[tokio::test]
    async fn test_monitor_task_no_key() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }));

        // Should return NotReady error when no key is set
        let result = task.get().await;
        assert!(matches!(result, None));
    }

    #[tokio::test]
    async fn test_monitor_task_target() {
        let task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }));
        let writer = Arc::new(Writer::<String>::new("127.0.0.1:11211"));

        let task_with_target = task.target(writer.clone());
        assert!(task_with_target.target.is_some());
    }

    #[tokio::test]
    async fn test_monitor_task_interval() {
        let task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }));
        let custom_interval = Duration::from_secs(10);

        let task_with_interval = task.interval(custom_interval);
        assert_eq!(task_with_interval.interval, custom_interval);
    }

    #[tokio::test]
    async fn test_monitor_task_key_method() {
        let task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }));

        let task_with_key = task.key("custom_key");
        assert_eq!(task_with_key.key, Some("custom_key".to_string()));
    }

    #[tokio::test]
    async fn test_monitor_task_chaining() {
        let writer = Arc::new(Writer::<String>::new("127.0.0.1:11211"));
        let custom_interval = Duration::from_secs(15);

        let task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .target(writer.clone())
            .interval(custom_interval)
            .key("chained_key");

        assert!(task.target.is_some());
        assert_eq!(task.interval, custom_interval);
        assert_eq!(task.key, Some("chained_key".to_string()));
    }

    #[tokio::test]
    async fn test_monitor_task_tick_expired() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .key("test_key");

        // Task should be expired initially (until not touched)
        assert!(task.has_expired());

        // Tick should return false for expired task
        let result = task.tick().await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_monitor_task_tick_not_expired() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .key("test_key");

        // Touch to make it non-expired
        task.touch();
        assert!(!task.has_expired());

        // Tick should return true for non-expired task
        let result = task.tick().await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_monitor_task_with_writer_target() {
        let writer = Arc::new(Writer::<String>::new("127.0.0.1:11211"));
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .target(writer)
            .key("test_key");

        task.touch();
        let result = task.get().await;
        assert_eq!(result, Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_monitor_tasks_new() {
        let tasks: MonitorTasks<String> = MonitorTasks::new();
        // Test that tick doesn't panic on empty tasks
        tasks.tick().await;
    }

    #[tokio::test]
    async fn test_monitor_task_get_updates_timestamp() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { Some("test_value".to_string()) }))
            .key("test_key");

        let initial_updated_at = task.updated_at;

        // Small delay to ensure timestamp difference
        sleep(Duration::from_millis(1)).await;

        task.touch();
        let _ = task.get().await;

        // updated_at should be different after get() call
        assert_ne!(task.updated_at, initial_updated_at);
    }

    #[tokio::test]
    async fn test_monitor_task_get_none_value() {
        let mut task = MonitorTask::new(|_key| Box::pin(async { None::<String> })).key("test_key");

        task.touch();
        let result = task.get().await;

        assert_eq!(result, None);
        assert_eq!(task.last_result(), None);
    }
}
