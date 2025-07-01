use thiserror::Error;

pub mod monitor;
pub mod protocol;
pub mod server;
pub mod service;
pub mod writer;

pub use monitor::MonitorTask;
pub use server::Server;
pub use service::{AsyncGetter, Service};
pub use writer::Writer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found")]
    NotFound,

    #[error("not ready")]
    NotReady,

    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
}
