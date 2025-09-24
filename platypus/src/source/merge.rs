use crate::{Response, Source, replace_placeholders, request::Request, response::MonitorConfig};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum RuleArgs {
    Inherit,

    Replace { args: HashMap<String, String> },
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub key: Vec<String>,
    pub source: String,
    pub args: RuleArgs,
}

impl Rule {
    pub fn new(key: Vec<String>, source: String, args: RuleArgs) -> Self {
        return Self { key, source, args };
    }
}

#[derive(Clone)]
pub struct Merge {
    monitor_config: MonitorConfig,

    // Format of output. Defaults to json
    format: String,

    rules: Vec<Rule>,
}

impl Deref for Merge {
    type Target = MonitorConfig;

    fn deref(&self) -> &Self::Target {
        &self.monitor_config
    }
}

impl DerefMut for Merge {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.monitor_config
    }
}

impl Merge {
    pub fn new() -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            format: "json".to_string(),
            rules: Vec::new(),
        }
    }

    pub fn with_format(mut self, format: &str) -> Self {
        self.format = format.to_string();
        self
    }

    pub fn with_rule(mut self, key: Vec<String>, source: String, args: RuleArgs) -> Self {
        let r = Rule::new(key, source, args);
        self.rules.push(r);
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

    // Helper method to set a value at a nested key path
    fn set_nested_value(
        map: &mut serde_json::Map<String, Value>,
        key_path: &[String],
        value: Value,
    ) {
        if key_path.is_empty() {
            return;
        }

        if key_path.len() == 1 {
            // Base case: set the value at this key
            map.insert(key_path[0].clone(), value);
        } else {
            // Recursive case: navigate/create nested structure
            let current_key = &key_path[0];
            let remaining_path = &key_path[1..];

            // Get or create the nested map
            let nested_map = map
                .entry(current_key.clone())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));

            // Ensure it's an object (map)
            if let Value::Object(nested) = nested_map {
                Self::set_nested_value(nested, remaining_path, value);
            }
        }
    }
}

#[async_trait]
impl Source for Merge {
    async fn call(&self, request: &Request) -> Response {
        let response = Response::new()
            .with_expiry(self.expiry())
            .with_ttl(self.ttl());

        let sources = match request.sources() {
            Some(sources) => sources,
            None => return response,
        };

        let mut merged_results = serde_json::Map::new();

        for rule in self.rules.iter() {
            if let Some(source) = sources.get(&rule.source) {
                // Get the appropriate request
                let replace_request = match &rule.args {
                    RuleArgs::Inherit => request,
                    RuleArgs::Replace { args } => {
                        // Create processed captures from rule args
                        let mut processed_captures = HashMap::new();
                        for (key, value) in args {
                            let replaced_value = replace_placeholders(value, request.captures());
                            processed_captures.insert(key.clone(), replaced_value);
                        }

                        // Create new request with processed captures
                        &Request::new(request.key())
                            .with_captures(processed_captures)
                            .with_sources(sources.clone())
                    }
                };

                // Call the source with appropriate request
                let source_response = source.call(replace_request).await;

                // If the source returned a value, place it at the nested key path
                if let Some(value) = source_response.value() {
                    // Parse the value as JSON, or treat as string if it fails
                    let json_value = match serde_json::from_str::<Value>(&value) {
                        Ok(parsed) => parsed,
                        Err(_) => Value::String(value),
                    };

                    // Place the value at the nested key path
                    Self::set_nested_value(&mut merged_results, &rule.key, json_value);
                }
            }
        }

        // Convert merged results to the requested format
        let output = match self.format.as_str() {
            "json" => serde_json::to_string(&merged_results).unwrap_or_default(),
            _ => serde_json::to_string(&merged_results).unwrap_or_default(), // Default to JSON
        };

        response.with_value(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Request, Sources, source::source};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    // Helper function to create a mock source that returns a fixed value
    fn create_mock_source(value: &str) -> Arc<Box<dyn Source>> {
        let value = value.to_string();
        Arc::new(Box::new(source(move |_key| {
            let value = value.clone();
            async move { Some(value) }
        })))
    }

    // Helper function to create a Sources HashMap
    fn create_test_sources() -> Arc<Sources> {
        let mut sources = HashMap::new();
        sources.insert("echo1".to_string(), create_mock_source("echo1 response"));
        sources.insert("echo2".to_string(), create_mock_source("echo2 response"));
        sources.insert("json_source".to_string(), create_mock_source(r#"{"nested": "value"}"#));
        Arc::new(sources)
    }

    #[test]
    fn test_merge_new() {
        let merge = Merge::new();
        assert_eq!(merge.format, "json");
        assert!(merge.rules.is_empty());
        // Default MonitorConfig has zero durations
        assert_eq!(merge.ttl(), Duration::from_secs(0));
        assert_eq!(merge.expiry(), Duration::from_secs(0));
    }

    #[test]
    fn test_merge_with_format() {
        let merge = Merge::new().with_format("xml");
        assert_eq!(merge.format, "xml");
    }

    #[test]
    fn test_merge_with_rule() {
        let merge = Merge::new()
            .with_rule(vec!["test".to_string()], "source1".to_string(), RuleArgs::Inherit);

        assert_eq!(merge.rules.len(), 1);
        assert_eq!(merge.rules[0].key, vec!["test".to_string()]);
        assert_eq!(merge.rules[0].source, "source1");
        matches!(merge.rules[0].args, RuleArgs::Inherit);
    }

    #[test]
    fn test_merge_with_ttl_and_expiry() {
        let ttl = Duration::from_secs(60);
        let expiry = Duration::from_secs(300);

        let merge = Merge::new()
            .with_ttl(ttl)
            .with_expiry(expiry);

        assert_eq!(merge.ttl(), ttl);
        assert_eq!(merge.expiry(), expiry);
    }

    #[test]
    fn test_deref_behavior() {
        let merge = Merge::new()
            .with_ttl(Duration::from_secs(120))
            .with_expiry(Duration::from_secs(600));

        // Test that we can access MonitorConfig methods through Deref
        assert_eq!(merge.ttl(), Duration::from_secs(120));
        assert_eq!(merge.expiry(), Duration::from_secs(600));
    }

    #[tokio::test]
    async fn test_merge_inherit_single_source() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_rule(vec!["result".to_string()], "echo1".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        assert_eq!(json["result"], "echo1 response");
    }

    #[tokio::test]
    async fn test_merge_replace_single_source() {
        let sources = create_test_sources();
        let mut replace_args = HashMap::new();
        replace_args.insert("path".to_string(), "test_value".to_string());

        let merge = Merge::new()
            .with_rule(vec!["result".to_string()], "echo1".to_string(),
                RuleArgs::Replace { args: replace_args });

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        assert_eq!(json["result"], "echo1 response");
    }

    #[tokio::test]
    async fn test_merge_multiple_sources() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_rule(vec!["first".to_string()], "echo1".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["second".to_string()], "echo2".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        assert_eq!(json["first"], "echo1 response");
        assert_eq!(json["second"], "echo2 response");
    }

    #[tokio::test]
    async fn test_merge_no_sources_in_request() {
        let merge = Merge::new()
            .with_rule(vec!["result".to_string()], "echo1".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key"); // No sources attached
        let response = merge.call(&request).await;

        // Should return empty response when no sources available
        assert!(response.value().is_none());
    }

    #[tokio::test]
    async fn test_merge_missing_source() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_rule(vec!["result".to_string()], "nonexistent".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        // Should produce empty JSON object when source doesn't exist
        assert!(json.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_set_nested_value_single_key() {
        let mut map = serde_json::Map::new();
        let key_path = vec!["test".to_string()];
        let value = serde_json::Value::String("value".to_string());

        Merge::set_nested_value(&mut map, &key_path, value);

        assert_eq!(map.get("test").unwrap(), "value");
    }

    #[test]
    fn test_set_nested_value_nested_keys() {
        let mut map = serde_json::Map::new();
        let key_path = vec!["level1".to_string(), "level2".to_string(), "level3".to_string()];
        let value = serde_json::Value::String("deep_value".to_string());

        Merge::set_nested_value(&mut map, &key_path, value);

        let level1 = map.get("level1").unwrap().as_object().unwrap();
        let level2 = level1.get("level2").unwrap().as_object().unwrap();
        assert_eq!(level2.get("level3").unwrap(), "deep_value");
    }

    #[test]
    fn test_set_nested_value_empty_path() {
        let mut map = serde_json::Map::new();
        let key_path = vec![];
        let value = serde_json::Value::String("value".to_string());

        Merge::set_nested_value(&mut map, &key_path, value);

        // Empty path should not modify the map
        assert!(map.is_empty());
    }

    #[test]
    fn test_set_nested_value_overwrite_existing() {
        let mut map = serde_json::Map::new();
        map.insert("existing".to_string(), serde_json::Value::String("old".to_string()));

        let key_path = vec!["existing".to_string()];
        let value = serde_json::Value::String("new".to_string());

        Merge::set_nested_value(&mut map, &key_path, value);

        assert_eq!(map.get("existing").unwrap(), "new");
    }

    #[tokio::test]
    async fn test_merge_nested_key_paths() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_rule(vec!["data".to_string(), "echo1".to_string()], "echo1".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["data".to_string(), "echo2".to_string()], "echo2".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        let data = json["data"].as_object().unwrap();
        assert_eq!(data["echo1"], "echo1 response");
        assert_eq!(data["echo2"], "echo2 response");
    }

    // Helper function to create sources that use request captures
    fn create_capture_aware_sources() -> Arc<Sources> {
        let mut sources: Sources = HashMap::new();

        // Create a source that returns a template using captures
        sources.insert("template_source".to_string(), Arc::new(Box::new(
            crate::source::Echo::new().with_template("path={path}, id={id}")
        ) as Box<dyn Source>));

        sources.insert("simple".to_string(), create_mock_source("simple response"));
        Arc::new(sources)
    }

    #[tokio::test]
    async fn test_replace_args_placeholder_processing() {
        let sources = create_capture_aware_sources();
        let mut replace_args = HashMap::new();
        replace_args.insert("path".to_string(), "{id}/data".to_string());  // Uses placeholder from request
        replace_args.insert("id".to_string(), "123".to_string());           // Static value

        let merge = Merge::new()
            .with_rule(vec!["result".to_string()], "template_source".to_string(),
                RuleArgs::Replace { args: replace_args });

        let mut captures = HashMap::new();
        captures.insert("id".to_string(), "456".to_string());
        let request = Request::new("test_key")
            .with_captures(captures)
            .with_sources(sources);

        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        // The {id} in "path={path}, id={id}" should be replaced with the processed args
        // path should be "456/data" (from replacing {id} with request capture)
        // id should be "123" (static value from replace args)
        assert_eq!(json["result"], "path=456/data, id=123");
    }

    #[tokio::test]
    async fn test_replace_args_multiple_placeholders() {
        let sources = create_capture_aware_sources();
        let mut replace_args = HashMap::new();
        replace_args.insert("path".to_string(), "{type}/{id}/details".to_string());
        replace_args.insert("id".to_string(), "fixed_id".to_string());

        let merge = Merge::new()
            .with_rule(vec!["result".to_string()], "template_source".to_string(),
                RuleArgs::Replace { args: replace_args });

        let mut captures = HashMap::new();
        captures.insert("type".to_string(), "user".to_string());
        captures.insert("id".to_string(), "789".to_string());
        let request = Request::new("test_key")
            .with_captures(captures)
            .with_sources(sources);

        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();
        // path should be "user/789/details", id should be "fixed_id"
        assert_eq!(json["result"], "path=user/789/details, id=fixed_id");
    }

    #[tokio::test]
    async fn test_inherit_vs_replace_behavior() {
        let sources = create_capture_aware_sources();
        let mut replace_args = HashMap::new();
        replace_args.insert("path".to_string(), "replaced_path".to_string());
        replace_args.insert("id".to_string(), "replaced_id".to_string());

        let merge = Merge::new()
            .with_rule(vec!["inherit".to_string()], "template_source".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["replace".to_string()], "template_source".to_string(),
                RuleArgs::Replace { args: replace_args });

        let mut captures = HashMap::new();
        captures.insert("path".to_string(), "original_path".to_string());
        captures.insert("id".to_string(), "original_id".to_string());
        let request = Request::new("test_key")
            .with_captures(captures)
            .with_sources(sources);

        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();

        // Inherit should use original captures
        assert_eq!(json["inherit"], "path=original_path, id=original_id");

        // Replace should use replaced values
        assert_eq!(json["replace"], "path=replaced_path, id=replaced_id");
    }

    #[tokio::test]
    async fn test_merge_json_responses() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_rule(vec!["json_data".to_string()], "json_source".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["string_data".to_string()], "echo1".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();

        // JSON source should be parsed and stored as object
        assert_eq!(json["json_data"], serde_json::json!({"nested": "value"}));
        // String source should be stored as string
        assert_eq!(json["string_data"], "echo1 response");
    }

    #[tokio::test]
    async fn test_merge_non_json_responses() {
        let mut sources: Sources = HashMap::new();
        sources.insert("plain_text".to_string(), create_mock_source("just plain text"));
        sources.insert("number_string".to_string(), create_mock_source("12345"));
        let sources = Arc::new(sources);

        let merge = Merge::new()
            .with_rule(vec!["text".to_string()], "plain_text".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["number".to_string()], "number_string".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();

        assert_eq!(json["text"], "just plain text");
        assert_eq!(json["number"], 12345); // "12345" is valid JSON and gets parsed as a number
    }

    #[tokio::test]
    async fn test_merge_mixed_json_and_string() {
        let mut sources: Sources = HashMap::new();
        sources.insert("json".to_string(), create_mock_source(r#"{"key": "value", "count": 42}"#));
        sources.insert("text".to_string(), create_mock_source("plain text"));
        sources.insert("invalid_json".to_string(), create_mock_source(r#"{"incomplete": json"#));
        let sources = Arc::new(sources);

        let merge = Merge::new()
            .with_rule(vec!["valid_json".to_string()], "json".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["plain".to_string()], "text".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["broken_json".to_string()], "invalid_json".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();

        // Valid JSON should be parsed as object
        assert_eq!(json["valid_json"], serde_json::json!({"key": "value", "count": 42}));
        // Plain text should be stored as string
        assert_eq!(json["plain"], "plain text");
        // Invalid JSON should fall back to string
        assert_eq!(json["broken_json"], r#"{"incomplete": json"#);
    }

    #[tokio::test]
    async fn test_merge_empty_responses() {
        let mut sources: Sources = HashMap::new();
        sources.insert("empty".to_string(), Arc::new(Box::new(
            source(move |_key| async move { None })
        ) as Box<dyn Source>));
        sources.insert("valid".to_string(), create_mock_source("has value"));
        let sources = Arc::new(sources);

        let merge = Merge::new()
            .with_rule(vec!["empty_result".to_string()], "empty".to_string(), RuleArgs::Inherit)
            .with_rule(vec!["valid_result".to_string()], "valid".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let json: serde_json::Value = serde_json::from_str(&response.value().unwrap()).unwrap();

        // Empty response should not be included in the result
        assert!(!json.as_object().unwrap().contains_key("empty_result"));
        // Valid response should be included
        assert_eq!(json["valid_result"], "has value");
    }

    #[tokio::test]
    async fn test_merge_json_format() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_format("json") // Explicitly set JSON format
            .with_rule(vec!["result".to_string()], "echo1".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let response_value = response.value().unwrap();

        // Should be valid JSON
        let json: serde_json::Value = serde_json::from_str(&response_value).unwrap();
        assert_eq!(json["result"], "echo1 response");
    }

    #[tokio::test]
    async fn test_merge_unknown_format() {
        let sources = create_test_sources();
        let merge = Merge::new()
            .with_format("unknown_format") // Unknown format should default to JSON
            .with_rule(vec!["result".to_string()], "echo1".to_string(), RuleArgs::Inherit);

        let request = Request::new("test_key").with_sources(sources);
        let response = merge.call(&request).await;

        assert!(response.value().is_some());
        let response_value = response.value().unwrap();

        // Should still be valid JSON (fallback behavior)
        let json: serde_json::Value = serde_json::from_str(&response_value).unwrap();
        assert_eq!(json["result"], "echo1 response");
    }
}
