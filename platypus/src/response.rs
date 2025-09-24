use crate::Value;
use std::ops::{Deref, DerefMut};
use tokio::time::{Duration, Instant};

#[derive(Default, Copy, Clone)]
pub struct MonitorConfig {
    // Duration key on the target should be kept for
    // This should be greater than interval.
    // Every usage of getter will keep the refresh monitor alive for this period of time
    ttl: Duration,

    // Refresh will keep running until this instant
    expiry: Duration,
}

impl MonitorConfig {
    pub fn new(ttl: Duration, expiry: Duration) -> Self {
        Self { ttl, expiry }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn with_expiry(mut self, expiry: Duration) -> Self {
        self.expiry = expiry;
        self
    }

    pub fn expiry(&self) -> Duration {
        self.expiry
    }
}

pub struct Response {
    monitor_config: MonitorConfig,

    value: Option<Value>,

    // Last time refresh occurred
    updated_at: Instant,
}

impl Response {
    pub fn new() -> Self {
        Self {
            monitor_config: MonitorConfig::default(),
            value: None,
            updated_at: Instant::now(),
        }
    }

    pub fn with_value(mut self, value: Value) -> Self {
        self.value = Some(value);
        self.updated_at = Instant::now();
        self
    }

    pub fn value(&self) -> Option<Value> {
        self.value.clone()
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn with_expiry(mut self, expiry: Duration) -> Self {
        self.expiry = expiry;
        self
    }

    pub fn expiry(&self) -> Duration {
        self.expiry
    }

    pub fn updated_at(&self) -> Instant {
        self.updated_at
    }
}

impl Deref for Response {
    type Target = MonitorConfig;

    fn deref(&self) -> &Self::Target {
        &self.monitor_config
    }
}

impl DerefMut for Response {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.monitor_config
    }
}
