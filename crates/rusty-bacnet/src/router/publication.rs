//! A successful startup result lost in asyncio only closes its own generation.
use super::application::Application;
use super::config::PortConfig;
use super::lifecycle::{Lifecycle, StartError};
use super::session::NativeSession;
use crate::py_async::{PutBack, Via};
use pyo3::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(super) struct Publication {
    owner: Lifecycle<NativeSession>,
    generation: AtomicU64,
    settled: AtomicBool,
    cleaning: AtomicBool,
    #[cfg(test)]
    completed: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl Publication {
    pub fn new(owner: Lifecycle<NativeSession>) -> Arc<Self> {
        Arc::new(Self {
            owner,
            generation: AtomicU64::new(0),
            settled: AtomicBool::new(false),
            cleaning: AtomicBool::new(false),
            #[cfg(test)]
            completed: std::sync::Mutex::new(None),
        })
    }
    pub async fn start(
        &self,
        reenter: bool,
        configs: Vec<PortConfig>,
        application: Application,
    ) -> Result<(), StartError<bacnet_types::error::Error, bacnet_network::router::RouterStopError>>
    {
        let receipt = self
            .owner
            .start(reenter, move || NativeSession::new(configs, application))
            .await?;
        receipt.publish(|generation| self.arm(generation));
        Ok(())
    }
    fn arm(&self, generation: u64) {
        self.generation.store(generation, Ordering::SeqCst);
    }
    fn cleanup(&self) {
        let generation = self.generation.load(Ordering::SeqCst);
        if generation != 0 && !self.cleaning.swap(true, Ordering::SeqCst) {
            let owner = self.owner.clone();
            #[cfg(test)]
            let completed = self.completed.lock().unwrap().take();
            crate::py_async::spawn_cleanup(async move {
                let _ = owner.close_generation(generation).await;
                #[cfg(test)]
                if let Some(completed) = completed {
                    let _ = completed.send(());
                }
            });
        }
    }
}
impl PutBack for Publication {
    fn put_back(&self, _: Python<'_>, _: Py<PyAny>, _: Via) {
        self.cleanup();
    }
    fn settled(&self) {
        self.settled.store(true, Ordering::SeqCst);
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        if !self.settled.load(Ordering::SeqCst) {
            self.cleanup();
        }
    }
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
