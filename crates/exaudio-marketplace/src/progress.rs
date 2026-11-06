use crate::{Error, Result};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Preparing,
    Downloading,
    Extracting,
    Activating,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub stage: Stage,
    pub extension: Option<String>,
    /// Bytes for Downloading, entries for Extracting, packages for Preparing/Completed.
    pub completed: u64,
    pub total: Option<u64>,
}

struct Inner {
    cancelled: AtomicBool,
    notify: tokio::sync::Notify,
    observer: Box<dyn Fn(Progress) + Send + Sync>,
}

/// Independent cancellation and nested progress. Observers should enqueue events rather
/// than block. Cancellation interrupts network waits; extraction checks every 16 KiB.
/// The final atomic pointer replacement is the non-cancellable commit boundary.
#[derive(Clone)]
pub struct OperationControl {
    inner: Arc<Inner>,
}

impl Default for OperationControl {
    fn default() -> Self {
        Self::new(|_| {})
    }
}

impl OperationControl {
    pub fn new(observer: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
                notify: tokio::sync::Notify::new(),
                observer: Box::new(observer),
            }),
        }
    }
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            self.emit(Stage::Cancelled, None, 0, None);
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
    pub(crate) fn emit(
        &self,
        stage: Stage,
        extension: Option<&str>,
        completed: u64,
        total: Option<u64>,
    ) {
        (self.inner.observer)(Progress {
            stage,
            extension: extension.map(str::to_owned),
            completed,
            total,
        });
    }
}
