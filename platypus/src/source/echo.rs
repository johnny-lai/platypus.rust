use crate::{Request, Response, replace_placeholders, response::MonitorConfig, source::Source};
use async_trait::async_trait;
use std::ops::{Deref, DerefMut};

pub struct Echo {
    monitor_config: MonitorConfig,
    template: String,
}

impl Deref for Echo {
    type Target = MonitorConfig;

    fn deref(&self) -> &Self::Target {
        &self.monitor_config
    }
}

impl DerefMut for Echo {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.monitor_config
    }
}

impl Echo {
    pub fn new() -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            template: String::new(),
        }
    }

    pub fn template(&self) -> &str {
        self.template.as_str()
    }

    pub fn with_template(mut self, template: &str) -> Self {
        self.template = template.to_string();
        self
    }
}

#[async_trait]
impl Source for Echo {
    async fn call(&self, request: &Request) -> Response {
        let response = Response::new()
            .with_expiry(self.expiry())
            .with_ttl(self.ttl());

        let value = replace_placeholders(self.template(), request.captures());
        response.with_value(value)
    }
}
