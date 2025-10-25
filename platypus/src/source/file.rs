use crate::{Request, Response, replace_placeholders, response::MonitorConfig, source::Source};
use async_trait::async_trait;
use std::ops::{Deref, DerefMut};
use std::time::Duration;
use tracing;

pub struct File {
    monitor_config: MonitorConfig,
    path_template: String,
}

impl Deref for File {
    type Target = MonitorConfig;

    fn deref(&self) -> &Self::Target {
        &self.monitor_config
    }
}

impl DerefMut for File {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.monitor_config
    }
}

impl File {
    pub fn new(path_template: &str) -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            path_template: path_template.to_string(),
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

    fn build_path(&self, request: &Request) -> String {
        replace_placeholders(&self.path_template, request.captures())
    }
}

#[async_trait]
impl Source for File {
    async fn call(&self, request: &Request) -> Response {
        let response = Response::new()
            .with_expiry(self.expiry())
            .with_ttl(self.ttl());

        let path = self.build_path(request);

        match tokio::fs::read_to_string(&path).await {
            Ok(contents) => response.with_value(contents),
            Err(e) => {
                tracing::error!("Failed to read file '{}': {}", path, e);
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
    use tokio::fs;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn test_file_new() {
        let file = File::new("/data/{key}.txt");
        assert_eq!(file.path_template, "/data/{key}.txt");
    }

    #[test]
    fn test_file_with_ttl_and_expiry() {
        let ttl = Duration::from_secs(60);
        let expiry = Duration::from_secs(300);

        let file = File::new("/data/{key}.txt")
            .with_ttl(ttl)
            .with_expiry(expiry);

        assert_eq!(file.ttl(), ttl);
        assert_eq!(file.expiry(), expiry);
    }

    #[test]
    fn test_build_path_simple() {
        let file = File::new("/data/{$key}.txt");
        let request = Request::new("test_file");

        let path = file.build_path(&request);
        assert_eq!(path, "/data/test_file.txt");
    }

    #[test]
    fn test_build_path_with_captures() {
        let file = File::new("/data/{environment}/{service}/{key}.json");
        let re = Regex::new(r"^(?<environment>[^/]+)/(?<service>[^/]+)/(?<key>.+)$").unwrap();
        let request = Request::match_regex(&re, "prod/api/config").unwrap();

        let path = file.build_path(&request);
        assert_eq!(path, "/data/prod/api/config.json");
    }

    #[test]
    fn test_build_path_missing_capture() {
        let file = File::new("/data/{environment}/{$key}.txt");
        let request = Request::new("simple_key");

        // Missing placeholders will be removed
        let path = file.build_path(&request);
        assert_eq!(path, "/data//simple_key.txt");
    }

    #[tokio::test]
    async fn test_file_call_success() {
        // Create a temporary file
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("platypus_test_file.txt");
        let mut file_handle = fs::File::create(&file_path).await.unwrap();
        file_handle.write_all(b"test content").await.unwrap();
        file_handle.sync_all().await.unwrap();
        drop(file_handle);

        // Create a File source pointing to the temp file
        let file = File::new(file_path.to_str().unwrap())
            .with_ttl(Duration::from_secs(60))
            .with_expiry(Duration::from_secs(300));

        let request = Request::new("test_key");
        let response = file.call(&request).await;

        // Verify response
        assert_eq!(response.ttl(), Duration::from_secs(60));
        assert_eq!(response.expiry(), Duration::from_secs(300));
        assert_eq!(response.value(), Some("test content".to_string()));

        // Cleanup
        fs::remove_file(file_path).await.unwrap();
    }

    #[tokio::test]
    async fn test_file_call_not_found() {
        let file = File::new("/nonexistent/path/{$key}.txt")
            .with_ttl(Duration::from_secs(60))
            .with_expiry(Duration::from_secs(300));

        let request = Request::new("test_key");
        let response = file.call(&request).await;

        // Should return empty response on error
        assert_eq!(response.ttl(), Duration::from_secs(60));
        assert_eq!(response.expiry(), Duration::from_secs(300));
        assert_eq!(response.value(), None);
    }

    #[tokio::test]
    async fn test_file_call_with_template() {
        // Create temp directory structure
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join("platypus_test_dir");
        fs::create_dir_all(&test_dir).await.unwrap();

        let file_path = test_dir.join("config.json");
        let mut file_handle = fs::File::create(&file_path).await.unwrap();
        file_handle.write_all(b"{\"key\": \"value\"}").await.unwrap();
        file_handle.sync_all().await.unwrap();
        drop(file_handle);

        // Create File source with template
        let template_path = test_dir.join("{$key}.json").to_str().unwrap().to_string();
        let file = File::new(&template_path);

        let request = Request::new("config");
        let response = file.call(&request).await;

        assert_eq!(response.value(), Some("{\"key\": \"value\"}".to_string()));

        // Cleanup
        fs::remove_file(file_path).await.unwrap();
        fs::remove_dir(test_dir).await.unwrap();
    }

    #[test]
    fn test_deref_behavior() {
        let file = File::new("/data/{key}.txt")
            .with_ttl(Duration::from_secs(120))
            .with_expiry(Duration::from_secs(600));

        // Test that we can access MonitorConfig methods through Deref
        assert_eq!(file.ttl(), Duration::from_secs(120));
        assert_eq!(file.expiry(), Duration::from_secs(600));
    }
}
