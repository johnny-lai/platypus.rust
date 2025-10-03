use std::collections::HashMap;
use std::pin::Pin;
use thiserror::Error;

pub mod monitor;
pub mod pool;
pub mod protocol;
pub mod request;
pub mod response;
pub mod router;
pub mod server;
pub mod service;
pub mod source;
pub mod writer;

pub use monitor::{MonitorTask, MonitorTasks};
pub use pool::AwsSecretsManagerConnectionManager;
pub use request::Request;
pub use response::Response;
pub use router::Router;
pub use server::Server;
pub use service::Service;
pub use source::Source;
pub use source::Sources;
pub use writer::Writer;

pub use source::source;

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

pub type Value = String;

pub trait AsyncGetter:
    Fn(&Request) -> Pin<Box<dyn Future<Output = Response> + Send + '_>> + Clone + Send + Sync + 'static
{
}

impl<F> AsyncGetter for F where
    F: Fn(&Request) -> Pin<Box<dyn Future<Output = Response> + Send + '_>>
        + Clone
        + Send
        + Sync
        + 'static
{
}

pub fn replace_placeholders(text: &str, replacements: &HashMap<String, String>) -> String {
    let mut ret = String::new();
    let mut placeholders: Vec<String> = Vec::new();

    for (_i, ch) in text.chars().enumerate() {
        match ch {
            '{' => {
                placeholders.push(String::new());
            }
            '}' => {
                if let Some(placeholder) = placeholders.pop() {
                    match replacements.get(&placeholder) {
                        Some(replacement) => {
                            if let Some(parent) = placeholders.last_mut() {
                                parent.push_str(&replacement);
                            } else {
                                ret.push_str(&replacement);
                            }
                        }
                        None => {}
                    }
                }
            }
            _ => {
                if let Some(current) = placeholders.last_mut() {
                    current.push(ch);
                } else {
                    ret.push(ch);
                }
            }
        }
    }

    ret
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

    #[test]
    fn test_replace_placeholders() {
        let mut placeholders = HashMap::new();
        placeholders.insert("name".to_string(), "Alice".to_string());
        placeholders.insert("age".to_string(), "30".to_string());

        let template = "Hello {name}, you are {age} years old";
        let result = replace_placeholders(template, &placeholders);
        assert_eq!(result, "Hello Alice, you are 30 years old");
    }

    #[test]
    fn test_replace_placeholders_no_matches() {
        let placeholders = HashMap::new();
        let template = "Hello {name}";
        let result = replace_placeholders(template, &placeholders);
        assert_eq!(result, "Hello ");
    }

    #[test]
    fn test_replace_placeholders_multiple_occurrences() {
        let mut placeholders = HashMap::new();
        placeholders.insert("word".to_string(), "test".to_string());

        let template = "{word} {word} {word}";
        let result = replace_placeholders(template, &placeholders);
        assert_eq!(result, "test test test");
    }

    #[test]
    fn test_replace_placeholders_nested_occurrences() {
        let mut placeholders = HashMap::new();
        placeholders.insert("word".to_string(), "test".to_string());
        placeholders.insert("test".to_string(), "other".to_string());

        let template = "{{word}} {{word}} {{word}}";
        let result = replace_placeholders(template, &placeholders);
        assert_eq!(result, "other other other");
    }
}
