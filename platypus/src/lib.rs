use thiserror::Error;

pub mod monitor;
pub mod protocol;
pub mod server;
pub mod service;
pub mod writer;

pub use monitor::{MonitorTask, MonitorTasks};
pub use server::Server;
pub use service::Service;
use std::pin::Pin;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let not_found = Error::NotFound;
        assert_eq!(not_found.to_string(), "not found");

        let not_ready = Error::NotReady;
        assert_eq!(not_ready.to_string(), "not ready");

        let other = Error::Other(anyhow::anyhow!("custom error"));
        assert_eq!(other.to_string(), "other: custom error");
    }

    #[test]
    fn test_error_from_anyhow() {
        let anyhow_error = anyhow::anyhow!("test error");
        let error: Error = anyhow_error.into();
        match error {
            Error::Other(_) => {}
            _ => panic!("Expected Error::Other"),
        }
    }

    #[test]
    fn test_version_constant() {
        assert!(!VERSION.is_empty());
        assert!(VERSION.contains('.'));
    }

    #[test]
    fn test_async_getter_trait_exists() {
        // Just test that the trait exists and compiles
        #[allow(dead_code)]
        fn test_trait_bound<T: AsyncGetter>(_: T) {}
        // This test passes if the trait compiles correctly
    }
}
