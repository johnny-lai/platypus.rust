use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{Request, Response};
use async_trait::async_trait;

pub mod echo;
pub use echo::Echo;

pub mod http;
pub use http::Http;

pub mod merge;
pub use merge::Merge;

pub mod aws_secrets_manager;
pub use aws_secrets_manager::AwsSecretsManager;

#[async_trait]
pub trait Source: Send + Sync + 'static {
    async fn call(&self, request: &Request) -> Response;
}

pub type Sources = HashMap<String, Arc<Box<dyn Source>>>;

pub struct FnGetter<F> {
    func: F,
    ttl: Duration,
    expiry: Duration,
}

impl<F> FnGetter<F> {
    pub fn new(func: F) -> Self {
        Self {
            func,
            ttl: Duration::from_secs(5),
            expiry: Duration::from_secs(30),
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn with_expiry(mut self, expiry: Duration) -> Self {
        self.expiry = expiry;
        self
    }

    pub fn with_box(self) -> Box<Self> {
        Box::new(self)
    }
}

// #[async_trait]
// impl<F> Source for FnGetter<F>
// where
//     F: Fn(&Request) -> Response + Send + Sync + 'static,
// {
//     async fn call(&self, request: &Request) -> Response {
//         (self.func)(request)
//     }
// }

#[async_trait]
impl<F, Fut> Source for FnGetter<F>
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Option<String>> + Send + 'static,
{
    async fn call(&self, request: &Request) -> Response {
        let response = Response::new().with_expiry(self.expiry).with_ttl(self.ttl);
        if let Some(value) = (self.func)(request.key().to_string()).await {
            response.with_value(value)
        } else {
            response
        }
    }
}

// Create a source object from a function
pub fn source<F, Fut>(func: F) -> FnGetter<F>
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Option<String>> + Send + 'static,
{
    FnGetter::new(func)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_use_source() {
        let source = source(|key| async move { Some(format!("value for {}", key)) });
        let request = Request::new("test_key");
        let response = source.call(&request).await;
        assert_eq!(response.value(), Some("value for test_key".to_string()));
    }
}
