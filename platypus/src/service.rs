use crate::protocol::{self, Command, Item, Response};
use crate::{MonitorTask, Writer};
use anyhow::Result;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::Mutex;
use tokio::time::Duration;
use tower;
use tracing::info;

pub trait AsyncGetter:
    Fn(&str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>>
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<F> AsyncGetter for F where
    F: Fn(&str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>>
        + Clone
        + Send
        + Sync
        + 'static
{
}

#[derive(Clone)]
pub struct Service<F>
where
    F: AsyncGetter,
{
    getter: Option<Arc<F>>,
    target_address: Option<String>,
    version: String,
    monitor_tasks: Arc<Mutex<HashMap<String, MonitorTask<String>>>>,
}

impl<F> tower::Service<protocol::CommandContext> for Service<F>
where
    F: AsyncGetter,
{
    type Response = protocol::Response;
    type Error = crate::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: protocol::CommandContext) -> Self::Future {
        let service = self.clone();
        Box::pin(async move {
            match service.handle_command(req.command).await {
                Ok(response) => Ok(response),
                Err(e) => Ok(protocol::Response::Error(e.to_string())),
            }
        })
    }
}

impl<F> Service<F>
where
    F: AsyncGetter,
{
    pub fn new() -> Self {
        Self {
            getter: None,
            target_address: None,
            version: "0.0.0".into(),
            monitor_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn getter(mut self, f: F) -> Self {
        self.getter = Some(Arc::new(f));
        self
    }

    pub fn target(mut self, target_address: &str) -> Self {
        self.target_address = Some(target_address.to_owned());
        self
    }

    pub fn version(mut self, version: &str) -> Self {
        self.version = version.into();
        self
    }

    async fn get_or_create_monitor_task(
        &self,
        key: &str,
        getter: &Arc<F>,
        target_writer: &Option<Arc<Writer<String>>>,
    ) -> Option<String> {
        let mut tasks = self.monitor_tasks.lock().await;

        // Check if we already have a MonitorTask for this key
        if let Some(ref mut task) = tasks.get_mut(key) {
            task.touch();

            // Return the last cached value
            match task.last_result() {
                Some(ret) => return Some(ret),
                _ => {}
            }
        }

        // Create new MonitorTask for this key
        let getter_clone = getter.clone();
        let mut monitor_task = MonitorTask::new(move |key: &str| getter_clone(key))
            .interval(Duration::from_secs(5))
            .key(key);

        if let Some(target_writer) = target_writer {
            monitor_task = monitor_task.target(target_writer.clone());
        }

        // Touch
        monitor_task.touch();

        // Get the current value or trigger a fresh computation
        let value = monitor_task.get().await;

        // Keep monitor_task
        tasks.insert(key.to_string(), monitor_task);
        value
    }

    async fn handle_command(&self, command: Command) -> anyhow::Result<Response> {
        let target_writer = self
            .target_address
            .as_ref()
            .map(|addr| Arc::new(Writer::<String>::new(addr.as_ref())));

        if let Some(getter) = &self.getter {
            match command {
                Command::Get(keys) => {
                    info!(keys = ?keys, "GET command");
                    let mut items = Vec::new();
                    for key in &keys {
                        match self
                            .get_or_create_monitor_task(key, getter, &target_writer)
                            .await
                        {
                            Some(value) => {
                                let item = Item {
                                    key: key.clone(),
                                    flags: 0,
                                    exptime: 0,
                                    data: value.into_bytes(),
                                    cas: None,
                                };
                                items.push(item);
                            }
                            None => {}
                        }
                    }
                    Ok(Response::Values(items))
                }
                Command::Gets(keys) => {
                    info!(keys = ?keys, "GETS command");
                    let mut items = Vec::new();
                    for key in &keys {
                        match self
                            .get_or_create_monitor_task(key, getter, &target_writer)
                            .await
                        {
                            Some(value) => {
                                let item = Item {
                                    key: key.clone(),
                                    flags: 0,
                                    exptime: 0,
                                    data: value.into_bytes(),
                                    cas: Some(12345),
                                };
                                items.push(item);
                            }
                            None => {}
                        }
                    }
                    Ok(Response::Values(items))
                }
                Command::Gat(exptime, keys) => {
                    info!(exptime = exptime, keys = ?keys, "GAT command");
                    let mut items = Vec::new();
                    for key in &keys {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime,
                            data: b"sample_value".to_vec(),
                            cas: None,
                        };
                        items.push(item);
                    }
                    Ok(Response::Values(items))
                }
                Command::Gats(exptime, keys) => {
                    info!(exptime = exptime, keys = ?keys, "GATS command");
                    let mut items = Vec::new();
                    for key in &keys {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime,
                            data: b"sample_value".to_vec(),
                            cas: Some(12345),
                        };
                        items.push(item);
                    }
                    Ok(Response::Values(items))
                }
                Command::MetaGet(key, flags) => {
                    info!(key = key, flags = ?flags, "META GET command");
                    if let Some(value) = self
                        .get_or_create_monitor_task(&key, getter, &target_writer)
                        .await
                    {
                        let item = Item {
                            key: key.clone(),
                            flags: 0,
                            exptime: 0,
                            data: value.into_bytes(),
                            cas: Some(12345),
                        };
                        Ok(Response::MetaValue(item, flags))
                    } else {
                        Ok(Response::MetaEnd)
                    }
                }
                Command::MetaNoOp => {
                    info!("META NOOP command");
                    Ok(Response::MetaNoOp)
                }
                Command::Version => {
                    info!("VERSION command");
                    Ok(Response::Version(self.version.clone()))
                }
                Command::Stats(arg) => {
                    info!(arg = ?arg, "STATS command");
                    let stats = vec![
                        ("version".to_string(), "0.1.0".to_string()),
                        ("curr_connections".to_string(), "1".to_string()),
                        ("total_connections".to_string(), "1".to_string()),
                        ("cmd_get".to_string(), "0".to_string()),
                        ("cmd_set".to_string(), "0".to_string()),
                    ];
                    Ok(Response::Stats(stats))
                }
                Command::Touch(key, exptime) => {
                    info!(key = key, exptime = exptime, "TOUCH command");
                    Ok(Response::Touched)
                }
                Command::Quit => {
                    info!("QUIT command - closing connection");
                    Ok(Response::Error("Connection should close".to_string()))
                }
            }
        } else {
            Err(anyhow::anyhow!("No getter function configured"))
        }
    }
}
