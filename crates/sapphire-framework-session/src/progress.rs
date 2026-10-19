//! When a session last moved bytes.
//!
//! Shared between the writer task and the read loop, which both mark it as they go.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::time::Instant;

/// The last moment the session wrote or read something.
///
/// Kept as milliseconds since a fixed start, in one atomic, so marking it is cheap enough
/// to do per frame and needs no lock.
#[derive(Clone)]
pub(crate) struct Progress {
    start: Instant,
    last_ms: Arc<AtomicU64>,
}

impl Progress {
    /// A mark set to now.
    pub(crate) fn new() -> Self {
        Self {
            start: Instant::now(),
            last_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Record that the session moved bytes just now.
    pub(crate) fn touch(&self) {
        let ms = self.start.elapsed().as_millis() as u64;
        self.last_ms.store(ms, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_touch_is_seen_by_every_clone() {
        let progress = Progress::new();
        let other = progress.clone();
        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        other.touch();
        assert_eq!(progress.last_ms.load(Ordering::Relaxed), 5_000);
    }
}
