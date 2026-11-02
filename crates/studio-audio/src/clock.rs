use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Deterministic clock for headless engines: time only moves on [`advance`](Self::advance).
/// Clones share the same time.
#[derive(Clone, Debug)]
pub struct ManualClock {
    origin: Instant,
    elapsed_nanos: Arc<AtomicU64>,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            elapsed_nanos: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn advance(&self, by: Duration) {
        let nanos = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        self.elapsed_nanos.fetch_add(nanos, Ordering::Relaxed);
    }

    pub fn now(&self) -> Instant {
        self.origin + Duration::from_nanos(self.elapsed_nanos.load(Ordering::Relaxed))
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Clock {
    System,
    Manual(ManualClock),
}

impl Clock {
    pub(crate) fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            Self::Manual(clock) => clock.now(),
        }
    }
}
