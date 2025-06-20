use memcache::{Stream, ToMemcacheValue};
use std::{fmt::Display, str::FromStr};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub mod protocol;

pub struct Monitor {
    interval: tokio::time::Duration,
    key: String,
}

impl Monitor {
    pub fn new(interval: tokio::time::Duration, key: String) -> Self {
        Self { interval, key }
    }

    pub fn spawn<F, R>(self, f: F) -> (CancellationToken, JoinHandle<()>)
    where
        F: Fn() -> R + Send + 'static,
        R: Display + ToMemcacheValue<Stream> + Send + 'static,
    {
        let cancel = CancellationToken::new();
        let backend = memcache::Client::connect("memcache://127.0.0.1:11213").unwrap();

        let cancel_for_loop = cancel.clone();
        let future = tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            loop {
                tokio::select! {
                    _ = cancel_for_loop.cancelled() => { break }

                    _ = interval.tick() => {
                        let ret = f();
                        _ = backend.set(self.key.as_str(), ret, 60);
                    }
                }
            }
            println!("monitor shut down");
        });

        (cancel.clone(), future)
    }
}
