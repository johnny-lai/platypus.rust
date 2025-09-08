use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server: Option<ServerConfig>,
    pub routes: Option<HashMap<String, RouteGroupConfig>>,
    pub source: Option<HashMap<String, SourceConfig>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub prefix: Option<String>,
    pub target: Option<TargetConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TargetConfig {
    String(String),
    Struct { host: String, port: u16 },
}

impl TargetConfig {
    pub fn to_url(&self) -> String {
        match self {
            TargetConfig::String(s) => s.clone(),
            TargetConfig::Struct { host, port } => format!("memcache://{}:{}", host, port),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteGroupConfig {
    pub prefix: String,
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteConfig {
    #[serde(rename = "match")]
    pub pattern: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceConfig {
    Http {
        url: String,
        action: Option<String>,
        query: Option<HashMap<String, String>>,
        headers: Option<HashMap<String, String>>,
        poll: Option<String>,
        expiry: Option<String>,
    },
    Awssm {
        key: String,
        poll: Option<String>,
        expiry: Option<String>,
    },
    Map {
        merge: Vec<MergeConfig>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MergeConfig {
    pub source: String,
    pub to: String,
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
