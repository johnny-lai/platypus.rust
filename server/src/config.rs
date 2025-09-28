use humantime::parse_duration;
use platypus::{
    Router, Source, source,
    source::{AwsSecretsManager, Echo, Http},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;
use std::fmt;
use std::sync::Arc;

//- Merge ---------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum MergeRuleArgsConfig {
    #[serde(rename = "inherit")]
    Inherit,

    #[serde(rename = "replace")]
    Replace {
        #[serde(flatten)]
        args: HashMap<String, String>,
    },
}

impl MergeRuleArgsConfig {
    pub fn to_rule_args(&self) -> source::merge::RuleArgs {
        match &self {
            MergeRuleArgsConfig::Inherit => source::merge::RuleArgs::Inherit {},
            MergeRuleArgsConfig::Replace { args } => {
                source::merge::RuleArgs::Replace { args: args.clone() }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MergeRuleConfig {
    pub key: Vec<String>,
    pub source: String,
    pub args: MergeRuleArgsConfig,
}

//- Source --------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceConfig {
    #[serde(rename = "aws_secrets_manager")]
    AwsSecretsManager {
        secret_id: String,
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
    Merge {
        format: String,
        template: Vec<MergeRuleConfig>,
        ttl: Option<String>,
        expiry: Option<String>,
    },
}

impl SourceConfig {
    pub fn to_source(&self) -> anyhow::Result<Box<dyn Source>> {
        match self {
            SourceConfig::AwsSecretsManager {
                secret_id,
                ttl,
                expiry,
            } => {
                let mut source = AwsSecretsManager::new(secret_id);

                if let Some(ttl_str) = ttl {
                    let ttl_duration = parse_duration(ttl_str)?;
                    source = source.with_ttl(ttl_duration);
                }

                if let Some(expiry_str) = expiry {
                    let expiry_duration = parse_duration(expiry_str)?;
                    source = source.with_expiry(expiry_duration);
                }

                Ok(Box::new(source))
            }
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
            SourceConfig::Merge {
                format,
                template,
                ttl,
                expiry,
            } => {
                let mut merge = source::Merge::new().with_format(format);

                for r in template.iter() {
                    merge = merge.with_rule(r.key.clone(), r.source.clone(), r.args.to_rule_args());
                }

                if let Some(ttl_str) = ttl {
                    let ttl_duration = parse_duration(ttl_str)?;
                    merge = merge.with_ttl(ttl_duration);
                }

                if let Some(expiry_str) = expiry {
                    let expiry_duration = parse_duration(expiry_str)?;
                    merge = merge.with_expiry(expiry_duration);
                }

                Ok(Box::new(merge))
            }
        }
    }
}

//- Route ---------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteGroupConfig {
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteConfig {
    #[serde(rename = "match")]
    pub pattern: String,

    #[serde(rename = "to")]
    pub source: String,
}

//- Service -------------------------------------------------------------------
#[derive(Default, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub routes: HashMap<String, RouteGroupConfig>,

    #[serde(rename = "source")]
    pub source_configs: HashMap<String, SourceConfig>,
}

impl ServerConfig {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let config = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(config::Environment::with_prefix("PLATYPUS").separator("_"))
            .build()?;

        let config = config.try_deserialize::<ServerConfig>()?;
        Ok(config)
    }

    pub fn to_router(&self) -> anyhow::Result<Router> {
        let mut router = Router::new();

        for (_name, route) in self.routes.iter() {
            for r in route.routes.iter() {
                router = router.route(&r.pattern, &r.source);
            }
        }

        Ok(router)
    }

    pub fn to_sources(&self) -> anyhow::Result<HashMap<String, Arc<Box<dyn Source>>>> {
        let mut sources = HashMap::new();
        for (name, config) in self.source_configs.iter() {
            sources.insert(name.clone(), Arc::new(config.to_source()?));
        }
        Ok(sources)
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("routes", &self.routes)
            .field("source", &self.source_configs)
            .finish()
    }
}
