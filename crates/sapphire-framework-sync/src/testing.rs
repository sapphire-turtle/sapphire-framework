//! Helpers for tests (feature `test-util`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hash::ContentHash;
use crate::hlc::Clock;
use crate::replica::ContentSource;

/// A clock the test moves by hand.
#[derive(Debug)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }

    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A content source backed by files in a temporary directory, named by hash.
#[derive(Debug)]
pub struct MapSource {
    dir: tempfile::TempDir,
}

impl Default for MapSource {
    fn default() -> Self {
        Self {
            dir: tempfile::tempdir().expect("create a temporary directory"),
        }
    }
}

impl MapSource {
    pub fn with(self, bytes: &[u8]) -> Self {
        let hash = ContentHash::of_bytes(bytes);
        std::fs::write(self.dir.path().join(hash.to_hex()), bytes).expect("write source content");
        self
    }
}

impl ContentSource for MapSource {
    fn locate(&self, hash: &ContentHash) -> Option<PathBuf> {
        let path = self.dir.path().join(hash.to_hex());
        path.is_file().then_some(path)
    }
}

/// Where an injected fault fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPoint {
    /// After a path state is committed, before its file is written or removed.
    AfterCommitBeforeWrite,
}
