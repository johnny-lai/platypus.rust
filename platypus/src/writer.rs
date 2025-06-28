use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

type WriteJob = Box<dyn FnOnce() + Send + 'static>;

pub struct Writer {
    sender: Sender<WriteJob>,
    shutdown_sender: Sender<()>,
    handle: JoinHandle<()>,
}

impl Writer {
    pub fn new() -> Self {
        let (tx, rx) = channel::<WriteJob>();
        let (shutdown_tx, shutdown_rx) = channel::<()>();

        let handle = std::thread::Builder::new()
            .name("platypus-memcache-writer".to_string())
            .spawn(move || {
                loop {
                    match rx.try_recv() {
                        Ok(job) => job(),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            // Check for shutdown signal
                            if shutdown_rx.try_recv().is_ok() {
                                // Process remaining jobs before shutting down
                                while let Ok(job) = rx.try_recv() {
                                    job();
                                }
                                break;
                            }
                            // Brief sleep to avoid busy waiting
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            // Process remaining jobs before shutting down
                            while let Ok(job) = rx.try_recv() {
                                job();
                            }
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn memcache writer thread");

        Writer {
            sender: tx,
            shutdown_sender: shutdown_tx,
            handle,
        }
    }

    pub fn send(&self, job: WriteJob) -> Result<(), std::sync::mpsc::SendError<WriteJob>> {
        self.sender.send(job)
    }

    pub fn shutdown(self) {
        // Signal shutdown
        let _ = self.shutdown_sender.send(());
        // Wait for thread to finish processing all jobs
        let _ = self.handle.join();
    }
}
