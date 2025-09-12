use crate::{Request, Response, replace_placeholders, response::MonitorConfig, source::Source};
use async_trait::async_trait;
use reqwest::{Client, Method};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::time::Duration;
use tracing;
use url::Url;

pub struct Http {
    monitor_config: MonitorConfig,
    url_template: String,
    client: Client,
    method: Method,
    headers: HashMap<String, String>,
    timeout: Duration,
}

impl Deref for Http {
    type Target = MonitorConfig;

    fn deref(&self) -> &Self::Target {
        &self.monitor_config
    }
}

impl DerefMut for Http {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.monitor_config
    }
}

impl Http {
    pub fn new(url_template: &str) -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            url_template: url_template.to_string(),
            client: Client::new(),
            method: Method::GET,
            headers: HashMap::new(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create HTTP client with timeout");
        self.client = client;
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_method(mut self, method: Method) -> Self {
        self.method = method;
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

    fn build_url(&self, request: &Request) -> Result<Url, url::ParseError> {
        let url_str = replace_placeholders(self.url_template.as_str(), request.captures());

        Url::parse(&url_str)
    }
}

#[async_trait]
impl Source for Http {
    async fn call(&self, request: &Request) -> Response {
        let response = Response::new()
            .with_expiry(self.expiry())
            .with_ttl(self.ttl());

        // Build the URL from the template
        let url = match self.build_url(request) {
            Ok(url) => url,
            Err(e) => {
                tracing::error!(
                    "Failed to build URL from template '{}': {}",
                    self.url_template,
                    e
                );
                return response;
            }
        };

        // Build the HTTP request
        let mut req_builder = self.client.request(self.method.clone(), url);

        // Add custom headers
        for (key, value) in &self.headers {
            req_builder = req_builder.header(key, value);
        }

        // Make the HTTP request
        match req_builder.send().await {
            Ok(http_response) => match http_response.text().await {
                Ok(body) => response.with_value(body),
                Err(e) => {
                    tracing::error!("Failed to read HTTP response body: {}", e);
                    response
                }
            },
            Err(e) => {
                tracing::error!("HTTP request failed: {}", e);
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
    use std::collections::HashMap;

    #[test]
    fn test_http_new() {
        let http = Http::new("https://api.example.com/data/{key}");
        assert_eq!(http.url_template, "https://api.example.com/data/{key}");
        assert_eq!(http.method, Method::GET);
        assert!(http.headers.is_empty());
        assert_eq!(http.timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_http_with_timeout() {
        let timeout = Duration::from_secs(10);
        let http = Http::new("https://api.example.com/data/{key}").with_timeout(timeout);
        assert_eq!(http.timeout, timeout);
    }

    #[test]
    fn test_http_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let http = Http::new("https://api.example.com/data/{key}").with_headers(headers.clone());
        assert_eq!(http.headers, headers);
    }

    #[test]
    fn test_http_with_method() {
        let http = Http::new("https://api.example.com/data/{key}").with_method(Method::POST);
        assert_eq!(http.method, Method::POST);
    }

    #[test]
    fn test_http_with_ttl_and_expiry() {
        let ttl = Duration::from_secs(60);
        let expiry = Duration::from_secs(300);

        let http = Http::new("https://api.example.com/data/{key}")
            .with_ttl(ttl)
            .with_expiry(expiry);

        assert_eq!(http.ttl(), ttl);
        assert_eq!(http.expiry(), expiry);
    }

    #[test]
    fn test_build_url_with_captures() {
        let http = Http::new("https://api.example.com/{instance}/metrics/{metric}");
        let re = Regex::new(r"^(?<instance>[^/]+)/(?<metric>.+)$").unwrap();
        let request = Request::match_regex(&re, "server1/cpu_usage").unwrap();

        let url = http.build_url(&request).unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.example.com/server1/metrics/cpu_usage"
        );
    }

    #[test]
    fn test_build_url_missing_capture() {
        let http = Http::new("https://api.example.com/{instance}/data/{key}");
        let request = Request::new("simple_key");

        // Should still work, missing placeholders will be removed
        let url = http.build_url(&request).unwrap();
        assert_eq!(url.as_str(), "https://api.example.com//data/");
    }

    #[test]
    fn test_build_url_invalid_template() {
        let http = Http::new("not_a_valid_url/{key}");
        let request = Request::new("test");

        let result = http.build_url(&request);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_call_integration() {
        // This test requires network access and an actual HTTP server
        // In a real scenario, you'd use a mock server like wiremock
        // For now, we'll test the structure but can't test actual HTTP calls

        let http = Http::new("https://httpbin.org/get")
            .with_ttl(Duration::from_secs(60))
            .with_expiry(Duration::from_secs(300));

        let request = Request::new("test_key");
        let response = http.call(&request).await;

        // The response should have the correct TTL and expiry
        assert_eq!(response.ttl(), Duration::from_secs(60));
        assert_eq!(response.expiry(), Duration::from_secs(300));

        // If the request succeeds, it should have a value
        // If it fails (no network), it should not have a value
        // Both are valid outcomes for this test
    }

    #[test]
    fn test_deref_behavior() {
        let http = Http::new("https://api.example.com/{key}")
            .with_ttl(Duration::from_secs(120))
            .with_expiry(Duration::from_secs(600));

        // Test that we can access MonitorConfig methods through Deref
        assert_eq!(http.ttl(), Duration::from_secs(120));
        assert_eq!(http.expiry(), Duration::from_secs(600));
    }
}
