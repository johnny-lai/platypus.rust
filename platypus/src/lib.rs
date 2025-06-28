use thiserror::Error;

pub mod monitor;
pub mod protocol;
pub mod server;
pub mod writer;

pub use monitor::MonitorTask;
pub use server::Server;
pub use writer::Writer;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found")]
    NotFound,

    #[error("not ready")]
    NotReady,

    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
}
