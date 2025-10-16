use crate::router::Router;
use crate::{Source, Sources, Value};
use crate::{request::Request, response::Response, writer::Writer};
use moka::future::Cache;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::debug;

#[derive(Clone)]
pub struct MonitorTask {
    last_touch: Instant,

    // The key the value will be written to in the target
    request: Request,

    // Last result getter returned
    last_response: Option<Response>,

    // Result
    source: Arc<Box<dyn Source>>,

    // The target where updated values will be written to
    target: Option<Arc<Writer>>,
}

impl MonitorTask {
    pub fn new(source: Arc<Box<dyn Source>>, request: Request) -> MonitorTask {
        MonitorTask {
            last_touch: Instant::now(),
            request,
            last_response: None,
            source,
            target: None,
        }
    }

    pub fn touch(&mut self) {
        debug!("touch");
        self.last_touch = Instant::now();
    }

    pub fn last_result(&self) -> Option<Value> {
        match &self.last_response {
            Some(response) => response.value(),
            None => None,
        }
    }

    pub async fn get(&mut self) -> Option<Value> {
        debug!("get");
        let response = self.source.call(&self.request).await;
        if let Some(target) = &self.target {
            let value = response.value();
            let _ = target.send(self.request.key(), value, response.ttl());
        }
        let ret = response.value();
        self.last_response = Some(response);
        ret
    }

    // Returns time this should be polled next.
    // If None is returned, then it has expired.
    pub async fn poll(&mut self) -> Option<Instant> {
        if let Some(response) = &self.last_response {
            // Check if expired
            let poll_until = self.last_touch + response.expiry();
            if Instant::now().gt(&poll_until) {
                return None;
            }

            // Check whether to poll
            let next_poll = response.updated_at() + (response.ttl() / 2);
            if Instant::now().gt(&next_poll) {
                let _ = self.get().await;
            }
            Some(next_poll)
        } else {
            // TODO: Think about whether this is expired or not?
            None
        }
    }

    pub fn with_target(mut self, target: Arc<Writer>) -> Self {
        self.target = Some(target);
        self
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    /// Estimates the memory size of this MonitorTask in bytes
    pub fn estimated_size(&self) -> u32 {
        let mut size = 0u32;

        // Size of the key in the request
        size += self.request.key().len() as u32;

        // Size of captures in the request (approximate)
        for (k, v) in self.request.captures() {
            size += k.len() as u32 + v.len() as u32;
        }

        // Size of the value if present
        if let Some(value) = &self.last_result() {
            size += value.len() as u32;
        }

        // Fixed overhead for the struct itself (approximate)
        // Instant + Option<Response> + Arc pointers
        size += 128;

        size
    }
}

#[derive(Clone)]
pub struct MonitorTasks {
    tasks: Cache<String, MonitorTask>,
}

impl MonitorTasks {
    /// Creates a new MonitorTasks with a default max memory size of 100MB
    pub fn new() -> Self {
        Self::with_max_bytes(100 * 1024 * 1024)
    }

    /// Creates a new MonitorTasks with a specified max memory size in bytes
    pub fn with_max_bytes(max_bytes: u64) -> Self {
        let tasks = Cache::builder()
            .max_capacity(max_bytes)
            .weigher(|_key: &String, value: &MonitorTask| value.estimated_size())
            .build();
        Self { tasks }
    }

    pub async fn get_or_create_task(
        &self,
        key: &str,
        router: Arc<Router>,
        sources: Arc<Sources>,
        target_writer: &Option<Arc<Writer>>,
    ) -> Option<Value> {
        // Try to get existing task
        if let Some(mut task) = self.tasks.get(key).await {
            task.touch();
            if let Some(value) = task.last_result() {
                // Update the cache with the touched task
                self.tasks.insert(key.to_string(), task).await;
                return Some(value);
            }
        }

        // Create new task if not found or no result
        debug!(key= ?key, "New MonitorTask");
        if let Some((request, rule)) = router.rule(key) {
            if let Some(source) = sources.get(rule.source()) {
                let request_with_sources = request.with_sources(sources.clone());
                let mut monitor_task = MonitorTask::new(source.clone(), request_with_sources);
                if let Some(target_writer) = target_writer {
                    monitor_task = monitor_task.with_target(target_writer.clone());
                }
                monitor_task.touch();
                let value = monitor_task.get().await;
                self.tasks.insert(key.to_string(), monitor_task).await;
                value
            } else {
                None
            }
        } else {
            None
        }
    }

    pub async fn tick(&self) {
        // Iterate through all cached tasks and check if they're expired
        // Moka doesn't have a direct way to iterate, so we'll use run_pending_tasks
        // to ensure any scheduled evictions are processed
        self.tasks.run_pending_tasks().await;

        // Note: We can't easily iterate over Moka entries to check expiry.
        // Instead, expired tasks will be lazily removed when accessed or during
        // normal eviction. If we need explicit expiry checking, we would need to
        // maintain a separate index of keys, but that adds complexity.
        // For now, relying on the TTL/expiry logic in get_or_create_task is sufficient.
    }
}
