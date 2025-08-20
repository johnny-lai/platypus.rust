use crate::router::Router;
use crate::{Source, Value};
use crate::{request::Request, response::Response, writer::Writer};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::debug;

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
}

#[derive(Clone)]
pub struct MonitorTasks {
    tasks: Arc<Mutex<HashMap<String, MonitorTask>>>,
}

impl MonitorTasks {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_or_create_task(
        &self,
        key: &str,
        router: Arc<Router>,
        target_writer: &Option<Arc<Writer>>,
    ) -> Option<Value> {
        let mut tasks = self.tasks.lock().await;

        if let Some(ref mut task) = tasks.get_mut(key) {
            task.touch();
            match task.last_result() {
                Some(ret) => return Some(ret),
                _ => {}
            }
        }

        debug!(key= ?key, "New MonitorTask");
        if let Some((request, rule)) = router.rule(key) {
            let mut monitor_task = MonitorTask::new(rule.source(), request);
            if let Some(target_writer) = target_writer {
                monitor_task = monitor_task.with_target(target_writer.clone());
            }
            monitor_task.touch();
            let value = monitor_task.get().await;
            tasks.insert(key.to_string(), monitor_task);
            value
        } else {
            None
        }
    }

    pub async fn tick(&self) {
        let mut tasks = self.tasks.lock().await;
        let mut remaining_tasks = HashMap::new();

        for (key, mut task) in tasks.drain() {
            if task.poll().await != None {
                // Task is still running, keep it
                remaining_tasks.insert(key, task);
            } else {
                // Tasks that return None are dropped (completed)
                debug!("Task expired");
            }
        }

        *tasks = remaining_tasks;
    }
}
