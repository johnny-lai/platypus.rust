use crate::Value;
use tokio::time::{Duration, Instant};

pub struct Response {
    value: Option<Value>,

    // Duration key on the target should be kept for
    // This should be greater than interval.
    // Every usage of getter will keep the refresh monitor alive for this period of time
    ttl: Duration,

    // Refresh will keep running until this instant
    expiry: Duration,

    // Last time refresh occurred
    updated_at: Instant,
}

impl Response {
    pub fn new() -> Self {
        Self {
            value: None,
            ttl: Duration::ZERO,
            expiry: Duration::ZERO,
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
