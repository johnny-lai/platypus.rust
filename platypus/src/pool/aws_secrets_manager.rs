use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::Client;
use r2d2::{ManageConnection, Pool};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AwsSecretsManagerConnectionManager {
    config: Option<aws_config::SdkConfig>,
}

impl AwsSecretsManagerConnectionManager {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(mut self, config: aws_config::SdkConfig) -> Self {
        self.config = Some(config);
        self
    }
}

impl Default for AwsSecretsManagerConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct AwsSecretsManagerConnection {
    client: Client,
}

impl AwsSecretsManagerConnection {
    pub fn client(&self) -> &Client {
        &self.client
    }
}

#[derive(Debug)]
pub enum AwsSecretsManagerError {
    ClientCreation(String),
}

impl fmt::Display for AwsSecretsManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AwsSecretsManagerError::ClientCreation(msg) => {
                write!(f, "Failed to create AWS client: {}", msg)
            }
        }
    }
}

impl std::error::Error for AwsSecretsManagerError {}

impl ManageConnection for AwsSecretsManagerConnectionManager {
    type Connection = AwsSecretsManagerConnection;
    type Error = AwsSecretsManagerError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        // Use provided config or create a new one
        let config = match &self.config {
            Some(config) => config.clone(),
            None => {
                // For synchronous connection creation, we need to use a runtime
                // This is a limitation of r2d2's synchronous interface with async AWS SDK
                let config_future = aws_config::defaults(BehaviorVersion::latest()).load();

                match tokio::runtime::Handle::try_current() {
                    Ok(handle) => {
                        // We're in an async context, use the current runtime
                        handle.block_on(config_future)
                    }
                    Err(_) => {
                        // We're not in an async context, create a new runtime
                        let rt = tokio::runtime::Runtime::new()
                            .map_err(|e| AwsSecretsManagerError::ClientCreation(e.to_string()))?;
                        rt.block_on(config_future)
                    }
                }
            }
        };

        let client = Client::new(&config);

        Ok(AwsSecretsManagerConnection { client })
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        // For AWS clients, we can consider them always valid once created
        // The actual validation happens during API calls
        // If we wanted more thorough validation, we could make a lightweight API call here
        let _client = &conn.client;
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        // AWS SDK clients don't really "break" in the traditional sense
        // They're stateless HTTP clients that handle retries internally
        // We'll return false here since connections don't maintain persistent state
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use r2d2::Pool;

    #[test]
    fn test_connection_manager_creation() {
        let manager = AwsSecretsManagerConnectionManager::new();
        assert!(manager.config.is_none());
    }

    #[test]
    fn test_connection_manager_default() {
        let manager = AwsSecretsManagerConnectionManager::default();
        assert!(manager.config.is_none());
    }

    #[tokio::test]
    async fn test_pool_creation() {
        let manager = AwsSecretsManagerConnectionManager::new();

        // Create a pool with minimal configuration for testing
        let pool = Pool::builder()
            .max_size(1)
            .min_idle(Some(0))
            .test_on_check_out(false) // Disable validation for this test
            .build(manager);

        assert!(pool.is_ok());
    }

    // Connection creation test is skipped because it requires AWS credentials
    // and creates a runtime, which conflicts with async test contexts.
    // This is more appropriate as an integration test.
    #[test]
    #[ignore]
    fn test_connection_creation() {
        let manager = AwsSecretsManagerConnectionManager::new();

        // This test requires AWS credentials to be available
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok() || std::env::var("AWS_PROFILE").is_ok() {
            let connection = manager.connect();
            assert!(connection.is_ok());

            if let Ok(mut conn) = connection {
                let validation = manager.is_valid(&mut conn);
                assert!(validation.is_ok());

                let broken = manager.has_broken(&mut conn);
                assert!(!broken);
            }
        }
    }
}

//-----------------------------------------------------------------------------
// Pool Builder
//-----------------------------------------------------------------------------

pub struct AwsSecretsManagerPoolBuilder {
    max_size: u32,
    min_idle: Option<u32>,
    profile: Option<String>,
    region: Option<String>,
}

impl AwsSecretsManagerPoolBuilder {
    pub fn new() -> Self {
        Self {
            max_size: 2,
            min_idle: Some(0),
            profile: None,
            region: None,
        }
    }

    pub fn with_max_size(mut self, size: u32) -> Self {
        self.max_size = size;
        self
    }

    pub fn with_min_idle(mut self, min: u32) -> Self {
        self.min_idle = Some(min);
        self
    }

    pub fn with_profile(mut self, profile: &str) -> Self {
        self.profile = Some(profile.to_string());
        self
    }

    pub fn with_region(mut self, region: &str) -> Self {
        self.region = Some(region.to_string());
        self
    }

    pub async fn build(self) -> anyhow::Result<Arc<Pool<AwsSecretsManagerConnectionManager>>> {
        // Build AWS config with optional profile and region
        let mut config_loader = aws_config::defaults(BehaviorVersion::latest());

        if let Some(profile) = &self.profile {
            config_loader = config_loader.profile_name(profile);
        }

        if let Some(region) = &self.region {
            config_loader = config_loader.region(aws_config::Region::new(region.clone()));
        }

        let config = config_loader.load().await;

        // Create connection manager with the config
        let manager = AwsSecretsManagerConnectionManager::new().with_config(config);

        // Build the pool
        let pool = Pool::builder()
            .max_size(self.max_size)
            .min_idle(self.min_idle)
            .build(manager)
            .map_err(|e| anyhow::anyhow!("Failed to create connection pool: {}", e))?;

        Ok(Arc::new(pool))
    }
}

impl Default for AwsSecretsManagerPoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod pool_builder_tests {
    use super::*;

    #[test]
    fn test_pool_builder_defaults() {
        let builder = AwsSecretsManagerPoolBuilder::new();
        assert_eq!(builder.max_size, 2);
        assert_eq!(builder.min_idle, Some(0));
        assert!(builder.profile.is_none());
        assert!(builder.region.is_none());
    }

    #[test]
    fn test_pool_builder_with_methods() {
        let builder = AwsSecretsManagerPoolBuilder::new()
            .with_max_size(10)
            .with_min_idle(1)
            .with_profile("test-profile")
            .with_region("us-east-1");

        assert_eq!(builder.max_size, 10);
        assert_eq!(builder.min_idle, Some(1));
        assert_eq!(builder.profile, Some("test-profile".to_string()));
        assert_eq!(builder.region, Some("us-east-1".to_string()));
    }

    #[tokio::test]
    async fn test_pool_builder_build() {
        let builder = AwsSecretsManagerPoolBuilder::new();
        let pool = builder.build().await;

        // Pool should be created successfully even without AWS credentials
        // The actual connection happens lazily
        assert!(pool.is_ok());

        if let Ok(pool) = pool {
            assert_eq!(pool.max_size(), 2);
        }
    }
}
