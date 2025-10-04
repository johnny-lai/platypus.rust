use crate::pool::aws_secrets_manager::{
    AwsSecretsManagerConnectionManager, AwsSecretsManagerPoolBuilder,
};
use crate::{Request, Response, replace_placeholders, response::MonitorConfig, source::Source};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use r2d2::Pool;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;
use tracing;

pub struct AwsSecretsManager {
    monitor_config: MonitorConfig,
    secret_id_template: String,
    pool: Arc<Pool<AwsSecretsManagerConnectionManager>>,
}

impl Deref for AwsSecretsManager {
    type Target = MonitorConfig;

    fn deref(&self) -> &Self::Target {
        &self.monitor_config
    }
}

impl DerefMut for AwsSecretsManager {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.monitor_config
    }
}

impl AwsSecretsManager {
    /// Create a new AwsSecretsManager with the provided pool
    pub fn new(pool: Arc<Pool<AwsSecretsManagerConnectionManager>>) -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            secret_id_template: String::new(),
            pool,
        }
    }

    pub fn with_secret_id(mut self, template: &str) -> Self {
        self.secret_id_template = template.to_string();
        self
    }

    pub fn with_pool(mut self, pool: Arc<Pool<AwsSecretsManagerConnectionManager>>) -> Self {
        self.pool = pool;
        self
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.monitor_config = self.monitor_config.with_ttl(ttl);
        self
    }

    pub fn with_expiry(mut self, expiry: Duration) -> Self {
        self.monitor_config = self.monitor_config.with_expiry(expiry);
        self
    }

    fn build_secret_id(&self, request: &Request) -> String {
        replace_placeholders(&self.secret_id_template, request.captures())
    }
}

#[async_trait]
impl Source for AwsSecretsManager {
    async fn call(&self, request: &Request) -> Response {
        let response = Response::new()
            .with_expiry(self.expiry())
            .with_ttl(self.ttl());

        let secret_id = self.build_secret_id(request);

        // Get a connection from the pool
        let connection = match self.pool.get() {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("Failed to get connection from pool: {}", e);
                return response;
            }
        };

        let client = connection.client();

        match client.get_secret_value().secret_id(&secret_id).send().await {
            Ok(secret_response) => {
                if let Some(secret_string) = secret_response.secret_string() {
                    response.with_value(secret_string.to_string())
                } else if let Some(secret_binary) = secret_response.secret_binary() {
                    // Convert binary to base64 string for text-based cache storage
                    let encoded = general_purpose::STANDARD.encode(secret_binary.as_ref());
                    response.with_value(encoded)
                } else {
                    tracing::warn!("Secret '{}' has no string or binary value", secret_id);
                    response
                }
            }
            Err(e) => {
                tracing::error!("Failed to retrieve secret '{}': {}", secret_id, e);
                response
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Request;
    use regex::Regex;
    use std::time::Duration;

    #[tokio::test]
    async fn test_aws_secrets_manager_new() {
        let pool = AwsSecretsManagerPoolBuilder::new()
            .build()
            .await
            .expect("Failed to create pool");

        let awssm = AwsSecretsManager::new(pool).with_secret_id("myapp/{key}");
        assert_eq!(awssm.secret_id_template, "myapp/{key}");
        assert_eq!(awssm.pool.max_size(), 2); // Default max_size
    }

    #[tokio::test]
    async fn test_aws_secrets_manager_with_ttl_and_expiry() {
        let ttl = Duration::from_secs(120);
        let expiry = Duration::from_secs(600);

        let pool = AwsSecretsManagerPoolBuilder::new()
            .build()
            .await
            .expect("Failed to create pool");

        let awssm = AwsSecretsManager::new(pool)
            .with_secret_id("myapp/{key}")
            .with_ttl(ttl)
            .with_expiry(expiry);

        assert_eq!(awssm.ttl(), ttl);
        assert_eq!(awssm.expiry(), expiry);
    }

    #[tokio::test]
    async fn test_build_secret_id_simple() {
        let pool = AwsSecretsManagerPoolBuilder::new()
            .build()
            .await
            .expect("Failed to create pool");

        let awssm = AwsSecretsManager::new(pool).with_secret_id("myapp/{$key}");
        let request = Request::new("database_password");

        let secret_id = awssm.build_secret_id(&request);
        assert_eq!(secret_id, "myapp/database_password");
    }

    #[tokio::test]
    async fn test_build_secret_id_with_captures() {
        let pool = AwsSecretsManagerPoolBuilder::new()
            .build()
            .await
            .expect("Failed to create pool");

        let awssm =
            AwsSecretsManager::new(pool).with_secret_id("myapp/{environment}/{service}/{key}");
        let re = Regex::new(r"^(?<environment>[^/]+)/(?<service>[^/]+)/(?<key>.+)$").unwrap();
        let request = Request::match_regex(&re, "prod/api/database_password").unwrap();

        let secret_id = awssm.build_secret_id(&request);
        assert_eq!(secret_id, "myapp/prod/api/database_password");
    }

    #[tokio::test]
    async fn test_build_secret_id_missing_capture() {
        let pool = AwsSecretsManagerPoolBuilder::new()
            .build()
            .await
            .expect("Failed to create pool");

        let awssm = AwsSecretsManager::new(pool).with_secret_id("myapp/{environment}/{$key}");
        let request = Request::new("simple_key");

        // Should still work, missing placeholders will be removed
        let secret_id = awssm.build_secret_id(&request);
        assert_eq!(secret_id, "myapp//simple_key");
    }

    #[tokio::test]
    async fn test_deref_behavior() {
        let pool = AwsSecretsManagerPoolBuilder::new()
            .build()
            .await
            .expect("Failed to create pool");

        let awssm = AwsSecretsManager::new(pool)
            .with_secret_id("myapp/{$key}")
            .with_ttl(Duration::from_secs(180))
            .with_expiry(Duration::from_secs(900));

        // Test that we can access MonitorConfig methods through Deref
        assert_eq!(awssm.ttl(), Duration::from_secs(180));
        assert_eq!(awssm.expiry(), Duration::from_secs(900));
    }

    #[tokio::test]
    async fn test_with_custom_pool() {
        let pool = AwsSecretsManagerPoolBuilder::new()
            .with_max_size(5)
            .with_min_idle(1)
            .build()
            .await
            .expect("Failed to create pool");

        // Create AwsSecretsManager without calling new() to avoid nested runtime
        let awssm = AwsSecretsManager {
            monitor_config: MonitorConfig::default(),
            secret_id_template: "myapp/{key}".to_string(),
            pool: pool.clone(),
        };

        assert_eq!(awssm.pool.max_size(), 5);
        assert_eq!(awssm.secret_id_template, "myapp/{key}");
    }

    // Note: Integration tests with actual AWS Secrets Manager would require
    // AWS credentials and real secrets, so they're omitted here.
    // In a real scenario, you might use localstack or aws-sdk-test for mocking.
}
