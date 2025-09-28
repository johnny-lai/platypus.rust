use crate::{Request, Response, replace_placeholders, response::MonitorConfig, source::Source};
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::Client;
use base64::{Engine as _, engine::general_purpose};
use std::ops::{Deref, DerefMut};
use std::time::Duration;
use tracing;

pub struct AwsSecretsManager {
    monitor_config: MonitorConfig,
    secret_id_template: String,
    client: Option<Client>,
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
    pub fn new(secret_id_template: &str) -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            secret_id_template: secret_id_template.to_string(),
            client: None,
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.monitor_config = self.monitor_config.with_ttl(ttl);
        self
    }

    pub fn with_expiry(mut self, expiry: Duration) -> Self {
        self.monitor_config = self.monitor_config.with_expiry(expiry);
        self
    }

    async fn get_client(&mut self) -> Result<&Client, aws_sdk_secretsmanager::Error> {
        if self.client.is_none() {
            let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
            self.client = Some(Client::new(&config));
        }

        Ok(self.client.as_ref().unwrap())
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

        // Create a mutable self to get the client
        let mut self_mut = Self {
            monitor_config: self.monitor_config.clone(),
            secret_id_template: self.secret_id_template.clone(),
            client: self.client.clone(),
        };

        let client = match self_mut.get_client().await {
            Ok(client) => client,
            Err(e) => {
                tracing::error!("Failed to create AWS Secrets Manager client: {}", e);
                return response;
            }
        };

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

    #[test]
    fn test_aws_secrets_manager_new() {
        let awssm = AwsSecretsManager::new("myapp/{key}");
        assert_eq!(awssm.secret_id_template, "myapp/{key}");
        assert!(awssm.client.is_none());
    }

    #[test]
    fn test_aws_secrets_manager_with_ttl_and_expiry() {
        let ttl = Duration::from_secs(120);
        let expiry = Duration::from_secs(600);

        let awssm = AwsSecretsManager::new("myapp/{key}")
            .with_ttl(ttl)
            .with_expiry(expiry);

        assert_eq!(awssm.ttl(), ttl);
        assert_eq!(awssm.expiry(), expiry);
    }

    #[test]
    fn test_build_secret_id_simple() {
        let awssm = AwsSecretsManager::new("myapp/{$key}");
        let request = Request::new("database_password");

        let secret_id = awssm.build_secret_id(&request);
        assert_eq!(secret_id, "myapp/database_password");
    }

    #[test]
    fn test_build_secret_id_with_captures() {
        let awssm = AwsSecretsManager::new("myapp/{environment}/{service}/{key}");
        let re = Regex::new(r"^(?<environment>[^/]+)/(?<service>[^/]+)/(?<key>.+)$").unwrap();
        let request = Request::match_regex(&re, "prod/api/database_password").unwrap();

        let secret_id = awssm.build_secret_id(&request);
        assert_eq!(secret_id, "myapp/prod/api/database_password");
    }

    #[test]
    fn test_build_secret_id_missing_capture() {
        let awssm = AwsSecretsManager::new("myapp/{environment}/{$key}");
        let request = Request::new("simple_key");

        // Should still work, missing placeholders will be removed
        let secret_id = awssm.build_secret_id(&request);
        assert_eq!(secret_id, "myapp//simple_key");
    }

    #[test]
    fn test_deref_behavior() {
        let awssm = AwsSecretsManager::new("myapp/{$key}")
            .with_ttl(Duration::from_secs(180))
            .with_expiry(Duration::from_secs(900));

        // Test that we can access MonitorConfig methods through Deref
        assert_eq!(awssm.ttl(), Duration::from_secs(180));
        assert_eq!(awssm.expiry(), Duration::from_secs(900));
    }

    // Note: Integration tests with actual AWS Secrets Manager would require
    // AWS credentials and real secrets, so they're omitted here.
    // In a real scenario, you might use localstack or aws-sdk-test for mocking.
}
