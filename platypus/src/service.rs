use crate::AsyncGetter;
use crate::protocol::{self, Command, Item, Response};
use crate::{MonitorTasks, Writer};
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower;
use tracing::info;

#[derive(Clone)]
pub struct Service<F>
where
    F: AsyncGetter,
{
    getter: Option<Arc<F>>,
    monitor_tasks: MonitorTasks<String>,
    target_writer: Option<Arc<Writer<String>>>,
    version: String,
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
        Self::with_monitor_tasks(MonitorTasks::new())
    }

    pub fn with_monitor_tasks(monitor_tasks: MonitorTasks<String>) -> Self {
        Self {
            getter: None,
            monitor_tasks,
            target_writer: None,
            version: "0.0.0".into(),
        }
    }

    pub fn getter(mut self, f: F) -> Self {
        self.getter = Some(Arc::new(f));
        self
    }

    pub fn target(mut self, target_address: &str) -> Self {
        self.target_writer = Some(Arc::new(Writer::<String>::new(target_address)));
        self
    }

    pub fn version(mut self, version: &str) -> Self {
        self.version = version.into();
        self
    }

    async fn get_or_create_monitor_task(&self, key: &str, getter: &Arc<F>) -> Option<String> {
        self.monitor_tasks
            .get_or_create_task(key, getter, &self.target_writer)
            .await
    }

    async fn handle_command(&self, command: Command) -> anyhow::Result<Response> {
        if let Some(getter) = &self.getter {
            match command {
                Command::Get(keys) => {
                    info!(keys = ?keys, "GET command");
                    let mut items = Vec::new();
                    for key in &keys {
                        match self.get_or_create_monitor_task(key, getter).await {
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
                        match self.get_or_create_monitor_task(key, getter).await {
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
                    if let Some(value) = self.get_or_create_monitor_task(&key, getter).await {
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

    pub async fn tick(&self) {
        self.monitor_tasks.tick().await;
    }

    pub fn monitor_tasks(&self) -> &MonitorTasks<String> {
        &self.monitor_tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_tasks_exists() {
        // Just test that MonitorTasks can be created
        let _monitor_tasks: MonitorTasks<String> = MonitorTasks::new();
    }
}
