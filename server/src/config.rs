use humantime::parse_duration;
use platypus::{
    Router, Source,
    source::{Echo, Http},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Config {
    pub routes: HashMap<String, RouteGroupConfig>,
    pub source: HashMap<String, SourceConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteGroupConfig {
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
    Awssm {
        key: String,
        ttl: Option<String>,
        expiry: Option<String>,
    },
    Echo {
        template: String,
    },
    Http {
        url: String,
        method: Option<String>,
        query: Option<HashMap<String, String>>,
        headers: Option<HashMap<String, String>>,
        ttl: Option<String>,
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
        let config = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(config::Environment::with_prefix("PLATYPUS").separator("_"))
            .build()?;

        Ok(config.try_deserialize::<Config>()?)
    }

    pub fn to_source(&self, source: &str) -> anyhow::Result<Box<dyn Source>> {
        let config = self
            .source
            .get(source)
            .ok_or_else(|| anyhow::anyhow!("Source '{}' not found", source))?;

        match config {
            SourceConfig::Awssm { .. } => Err(anyhow::anyhow!("AWSSM source not implemented yet")),
            SourceConfig::Echo { template } => {
                let echo = Echo::new().with_template(template);
                Ok(Box::new(echo))
            }
            SourceConfig::Http {
                url,
                method,
                query: _,
                headers,
                ttl,
                expiry,
            } => {
                let mut http = Http::new(url);

                if let Some(method_str) = method {
                    let method = method_str
                        .parse()
                        .map_err(|_| anyhow::anyhow!("Invalid HTTP method: {}", method_str))?;
                    http = http.with_method(method);
                }

                if let Some(headers_map) = headers {
                    http = http.with_headers(headers_map.clone());
                }

                if let Some(ttl_str) = ttl {
                    let ttl_duration = parse_duration(ttl_str)?;
                    http = http.with_ttl(ttl_duration);
                }

                if let Some(expiry_str) = expiry {
                    let expiry_duration = parse_duration(expiry_str)?;
                    http = http.with_expiry(expiry_duration);
                }

                Ok(Box::new(http))
            }
            SourceConfig::Map { .. } => Err(anyhow::anyhow!("Map source not implemented yet")),
        }
    }

    pub fn to_router(&self) -> anyhow::Result<Router> {
        let mut router = Router::new();

        for (_name, route) in self.routes.iter() {
            for r in route.routes.iter() {
                let source = self.to_source(r.source.as_str())?;
                router = router.route(&r.pattern, source);
            }
        }

        Ok(router)
    }
}
