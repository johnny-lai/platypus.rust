use std::fmt::Display;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub mod protocol;

pub struct Monitor {
    interval: tokio::time::Duration,
}

impl Monitor {
    pub fn new(interval: tokio::time::Duration) -> Self {
        Self { interval }
    }

    pub fn spawn<F, R>(self, f: F) -> (CancellationToken, JoinHandle<()>)
    where
        F: Fn() -> R + Send + 'static,
        R: Display + Send + 'static,
    {
        let cancel = CancellationToken::new();

        let cancel_for_loop = cancel.clone();
        let future = tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            loop {
                tokio::select! {
                    _ = cancel_for_loop.cancelled() => { break }

                    _ = interval.tick() => {
                        let ret = f();
                        println!("got {}", ret);
                    }
                }
            }
            println!("monitor shut down");
        });

        (cancel.clone(), future)
    }
}
