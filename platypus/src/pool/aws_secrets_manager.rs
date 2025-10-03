use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::Client;
use r2d2::ManageConnection;
use std::fmt;

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
    ConfigLoad(aws_config::ConfigError),
    ClientCreation(String),
}

impl fmt::Display for AwsSecretsManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AwsSecretsManagerError::ConfigLoad(err) => {
                write!(f, "Failed to load AWS config: {}", err)
            }
            AwsSecretsManagerError::ClientCreation(msg) => {
                write!(f, "Failed to create AWS client: {}", msg)
            }
        }
    }
}

impl std::error::Error for AwsSecretsManagerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AwsSecretsManagerError::ConfigLoad(err) => Some(err),
            AwsSecretsManagerError::ClientCreation(_) => None,
        }
    }
}

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
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|e| AwsSecretsManagerError::ClientCreation(e.to_string()))?;

                rt.block_on(async {
                    aws_config::defaults(BehaviorVersion::latest())
                        .load()
                        .await
                })
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

    #[tokio::test]
    async fn test_connection_creation() {
        let manager = AwsSecretsManagerConnectionManager::new();

        // This test requires AWS credentials to be available
        // In CI/CD environments, you might want to skip this or use mocked credentials
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