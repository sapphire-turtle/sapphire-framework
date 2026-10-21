//! A replica: one workspace root kept in sync with peers by joining path states.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grain_id::GrainId;

use crate::entry::{Content, Entry, PathUpdate};
use crate::error::{Error, Result};
use crate::filter::SyncFilter;
use crate::hash::ContentHash;
use crate::hlc::Clock;
use crate::id::ReplicaId;
use crate::merge;
use crate::paths;
use crate::report::{Conflict, PauseReason, Report, ScanOutcome, SkipReason, Skipped};
use crate::staging::{self, Stager};
use crate::state::{DiskState, PathState};
use crate::store::{Meta, ReplicaStore};
use crate::vv::{Dot, VersionVector};

/// Files larger than this are not synced unless configured otherwise.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;

/// A file whose mtime is this close to when it was checked is re-hashed.
const RACY_NS: i64 = 2_000_000_000;

/// Path states held in memory before they are written in one transaction. A redb
/// commit costs several fsyncs, so writing each path on its own made a first sync
/// slow enough on a slow disk to outlast a peer's idle timeout (issue #305).
const MAX_PENDING: usize = 256;

/// Where a replica gets content it does not have: a peer, in practice.
///
/// Content is handed over by path so that it is never held in memory whole.
pub trait ContentSource {
    /// A file holding exactly the content `hash`, if there is one. The replica copies
    /// it through a verifying staging writer, so a wrong file is refused, not trusted.
    fn locate(&self, hash: &ContentHash) -> Option<PathBuf>;
}

/// How a replica is set up.
#[derive(Clone, Debug)]
pub struct ReplicaConfig {
    pub app_name: String,
    pub root: PathBuf,
    /// Recorded as `Entry::author` on local writes.
    pub device_id: GrainId,
    pub max_file_size: u64,
    pub store_path: PathBuf,
    pub staging_dir: PathBuf,
}

impl ReplicaConfig {
    /// A config keeping the store and staging directory under `state_dir`.
    pub fn new(
        app_name: impl Into<String>,
        root: impl Into<PathBuf>,
        device_id: GrainId,
        state_dir: &Path,
    ) -> Self {
        Self {
            app_name: app_name.into(),
            root: root.into(),
            device_id,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            store_path: state_dir.join("sync.redb"),
            staging_dir: state_dir.join("staging"),
        }
    }
}

/// One workspace root and its replica store.
pub struct Replica {
    config: ReplicaConfig,
    store: ReplicaStore,
    meta: Meta,
    clock: Arc<dyn Clock>,
    /// Path states committed but not yet written to the store; see [`Self::flush`].
    /// Every read of a state goes through [`Self::get_state`], which sees these first.
    pending: BTreeMap<String, PathState>,
    /// Whether `pending` holds a state with newly joined versions. Such a state is
    /// written before the workspace changes, so that a file placed from it is never
    /// newer than what the store knows: a crash would otherwise leave a peer's content
    /// on disk that the next scan records as a local edit.
    pending_versions: bool,
    #[cfg(any(test, feature = "test-util"))]
    fault: Option<crate::testing::FaultPoint>,
    /// Fires once after the next path a scan reconciles. Lets a test change the
    /// filesystem in the middle of a scan.
    ///
    /// `Send` so that a replica held across a `tokio::spawn` still is: the app server keeps
    /// one behind a mutex and moves it into a task per session.
    #[cfg(any(test, feature = "test-util"))]
    reconcile_hook: Option<Box<dyn FnOnce() + Send>>,
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn stamp_of(meta: &fs::Metadata) -> (i64, u64) {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    (mtime, meta.len())
}

/// How long staged content is kept whatever it holds; see [`Replica::collect_staging`].
pub const STAGING_GRACE: Duration = Duration::from_secs(60 * 60);

fn is_racy(disk: &DiskState) -> bool {
    disk.mtime_ns.saturating_add(RACY_NS) >= disk.checked_ns
}

/// Move a verified staged file into place, falling back to copy + rename when the
/// staging directory is on another volume.
///
/// The placement is durable when this returns: the caller commits the path's state next,
/// and a committed state whose file a crash then lost would read as a local delete on
/// the next scan — and spread to every peer as one.
fn place(staged: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(staged, dest) {
        Ok(()) => return sync_parent(dest),
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {}
        Err(e) => return Err(e.into()),
    }
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dest.with_file_name(format!(".{name}.sapphire-tmp"));
    let copied = fs::copy(staged, &tmp)
        .and_then(|_| fs::File::open(&tmp)?.sync_all())
        .and_then(|()| fs::rename(&tmp, dest));
    if let Err(e) = copied {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    sync_parent(dest)?;
    // The file is in place; the staged copy is a cache. Failing to remove it must not
    // turn a completed placement into an error and leave the state uncommitted.
    let _ = fs::remove_file(staged);
    Ok(())
}

/// Make a rename into `path`'s directory durable. Unix only: std cannot open a
/// directory to sync on Windows, where this is left to the filesystem.
fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Lookups over the stored path states that would otherwise be a full store read per
/// path: `case_twin` on a case-insensitive filesystem runs for every path of a first
/// scan, and `content_path` runs for every file materialized from a peer. Built once
/// per `scan`/`apply`/`fetch_missing` and kept current as states are committed, so it
/// never goes stale within a call.
#[derive(Debug, Default)]
struct StoreIndex {
    /// `disk.hash` of each stored path that has a file on disk.
    hash_of: HashMap<String, ContentHash>,
    /// `disk.hash` -> the stored paths whose file on disk has that hash, in path order.
    by_hash: HashMap<ContentHash, BTreeSet<String>>,
    /// Lowercased path -> the stored paths with that lowercase form. Left empty on a
    /// case-sensitive filesystem, where no path can have a case twin.
    by_lower: HashMap<String, BTreeSet<String>>,
}

impl StoreIndex {
    fn build(states: &[(String, PathState)]) -> Self {
        let mut index = Self::default();
        for (rel, state) in states {
            index.record(rel, state.disk.hash);
        }
        index
    }

    /// Note that `rel` now has `hash` on disk, replacing whatever it had before.
    fn record(&mut self, rel: &str, hash: Option<ContentHash>) {
        if paths::CASE_INSENSITIVE_FS {
            self.by_lower
                .entry(rel.to_lowercase())
                .or_default()
                .insert(rel.to_owned());
        }
        let previous = match hash {
            Some(hash) => self.hash_of.insert(rel.to_owned(), hash),
            None => self.hash_of.remove(rel),
        };
        if let Some(previous) = previous
            && Some(previous) != hash
            && let Some(paths) = self.by_hash.get_mut(&previous)
        {
            paths.remove(rel);
            if paths.is_empty() {
                self.by_hash.remove(&previous);
            }
        }
        if let Some(hash) = hash {
            self.by_hash.entry(hash).or_default().insert(rel.to_owned());
        }
    }

    /// Another stored path, differing from `rel` only in case, whose file is on disk.
    /// Always `None` on a case-sensitive filesystem.
    fn case_twin(&self, rel: &str) -> Option<String> {
        self.by_lower
            .get(&rel.to_lowercase())?
            .iter()
            .find(|other| other.as_str() != rel && self.hash_of.contains_key(*other))
            .cloned()
    }

    /// Stored paths whose file on disk has `hash`, in path order.
    fn paths_with(&self, hash: &ContentHash) -> impl Iterator<Item = &String> {
        self.by_hash.get(hash).into_iter().flatten()
    }
}

/// Turn a per-path `Error::Io` into a `Skipped` entry and let the caller continue;
/// propagate every other error so callers still abort on store/format/pause failures.
fn tolerate_io(result: Result<()>, rel: &str, report: &mut Report) -> Result<()> {
    match result {
        Err(Error::Io(e)) => {
            report.skipped.push(Skipped {
                path: rel.to_owned(),
                reason: SkipReason::Io(e.to_string()),
            });
            Ok(())
        }
        other => other,
    }
}

impl Replica {
    pub fn open(config: ReplicaConfig, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open_inner(config, clock, None)
    }

    fn open_inner(
        config: ReplicaConfig,
        clock: Arc<dyn Clock>,
        id: Option<ReplicaId>,
    ) -> Result<Self> {
        fs::create_dir_all(&config.staging_dir)?;
        staging::clear_partials(&config.staging_dir)?;
        let root = config.root.to_string_lossy().into_owned();
        let store = ReplicaStore::open_with_id(&config.store_path, &root, id)?;
        let meta = store.meta()?;
        let replica = Self {
            config,
            store,
            meta,
            clock,
            pending: BTreeMap::new(),
            pending_versions: false,
            #[cfg(any(test, feature = "test-util"))]
            fault: None,
            #[cfg(any(test, feature = "test-util"))]
            reconcile_hook: None,
        };
        replica.collect_staging_quietly();
        Ok(replica)
    }

    pub fn replica_id(&self) -> ReplicaId {
        self.meta.replica_id
    }

    pub fn vv(&self) -> &VersionVector {
        &self.meta.vv
    }

    pub fn config(&self) -> &ReplicaConfig {
        &self.config
    }

    pub fn state(&self, path: &str) -> Result<Option<PathState>> {
        self.get_state(path)
    }

    pub fn states(&self) -> Result<Vec<(String, PathState)>> {
        self.all_states()
    }

    /// The state of `path`, pending or stored.
    fn get_state(&self, path: &str) -> Result<Option<PathState>> {
        match self.pending.get(path) {
            Some(state) => Ok(Some(state.clone())),
            None => self.store.get(path),
        }
    }

    /// Every path state, pending or stored, in path order.
    fn all_states(&self) -> Result<Vec<(String, PathState)>> {
        let mut states = self.store.all()?;
        if self.pending.is_empty() {
            return Ok(states);
        }
        let mut pending = self.pending.clone();
        for (rel, state) in &mut states {
            if let Some(newer) = pending.remove(rel) {
                *state = newer;
            }
        }
        states.extend(pending);
        states.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(states)
    }

    /// Write the pending path states, and the metadata, in one transaction.
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let states: Vec<(String, PathState)> = self
            .pending
            .iter()
            .map(|(rel, state)| (rel.clone(), state.clone()))
            .collect();
        self.store.commit(&self.meta, &states)?;
        self.pending.clear();
        self.pending_versions = false;
        Ok(())
    }

    /// Flush before the workspace changes if a pending state holds joined versions.
    fn flush_versions(&mut self) -> Result<()> {
        if self.pending_versions {
            self.flush()?;
        }
        Ok(())
    }

    /// Flush what `result`'s call left pending, whether or not it failed: the files it
    /// already placed must be recorded. The call's own error takes precedence.
    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        let flushed = self.flush();
        let value = result?;
        flushed?;
        Ok(value)
    }

    fn filter(&self) -> Result<SyncFilter> {
        SyncFilter::load(&self.config.root, &self.config.app_name)
    }

    /// Why this replica must not scan or apply right now. A root or marker that is
    /// missing while files were written would otherwise read as "everything deleted".
    pub fn pause_reason(&self) -> Result<Option<PauseReason>> {
        let root_ok = self.config.root.is_dir();
        let marker_ok = self
            .config
            .root
            .join(format!(".{}", self.config.app_name))
            .is_dir();
        if root_ok && marker_ok {
            return Ok(None);
        }
        if !self
            .all_states()?
            .iter()
            .any(|(_, s)| s.disk.hash.is_some())
        {
            return Ok(None);
        }
        Ok(Some(if root_ok {
            PauseReason::MarkerMissing
        } else {
            PauseReason::RootMissing
        }))
    }

    #[cfg(any(test, feature = "test-util"))]
    fn fault_check(&mut self) -> Result<()> {
        use crate::testing::FaultPoint;
        if self.fault == Some(FaultPoint::AfterCommitBeforeWrite) {
            self.fault = None;
            return Err(Error::InjectedFault);
        }
        Ok(())
    }

    #[cfg(not(any(test, feature = "test-util")))]
    fn fault_check(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called after each file written or removed. A crash loses what is still pending.
    #[cfg(any(test, feature = "test-util"))]
    fn crash_check(&mut self) -> Result<()> {
        use crate::testing::FaultPoint;
        match self.fault {
            Some(FaultPoint::CrashAfterWrites(n)) if n > 1 => {
                self.fault = Some(FaultPoint::CrashAfterWrites(n - 1));
            }
            Some(FaultPoint::CrashAfterWrites(_)) => {
                self.fault = None;
                self.pending.clear();
                self.pending_versions = false;
                return Err(Error::InjectedFault);
            }
            _ => {}
        }
        Ok(())
    }

    #[cfg(not(any(test, feature = "test-util")))]
    fn crash_check(&mut self) -> Result<()> {
        Ok(())
    }

    #[cfg(any(test, feature = "test-util"))]
    fn run_reconcile_hook(&mut self) {
        if let Some(hook) = self.reconcile_hook.take() {
            hook();
        }
    }

    #[cfg(not(any(test, feature = "test-util")))]
    fn run_reconcile_hook(&mut self) {}

    /// Record every unrecorded edit under the root and finish pending writes.
    pub fn scan(&mut self) -> Result<ScanOutcome> {
        if let Some(reason) = self.pause_reason()? {
            return Ok(ScanOutcome::Paused(reason));
        }
        let result = self.scan_root();
        let report = self.finish(result)?;
        self.collect_staging_quietly();
        Ok(ScanOutcome::Scanned(report))
    }

    fn scan_root(&mut self) -> Result<Report> {
        let filter = self.filter()?;
        let mut report = Report::default();
        let mut rels = BTreeSet::new();
        if self.config.root.is_dir() {
            let root = self.config.root.clone();
            let walker = walkdir::WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0
                        || paths::rel_from_native(&root, e.path())
                            .is_some_and(|rel| filter.allows(&rel, e.file_type().is_dir()))
                });
            for item in walker {
                // One unreadable directory or Windows-locked file must not abort the
                // whole scan: report it like any other per-path I/O error and keep
                // walking, which `walkdir` does for the entry's siblings.
                let item = match item {
                    Ok(item) => item,
                    Err(e) => {
                        let path = e
                            .path()
                            .and_then(|p| paths::rel_from_native(&root, p))
                            .unwrap_or_default();
                        report.skipped.push(Skipped {
                            path,
                            reason: SkipReason::Io(e.to_string()),
                        });
                        continue;
                    }
                };
                if item.depth() == 0 || item.file_type().is_dir() {
                    continue;
                }
                let Some(rel) = paths::rel_from_native(&root, item.path()) else {
                    continue;
                };
                if item.file_type().is_symlink() {
                    report.skipped.push(Skipped {
                        path: rel,
                        reason: SkipReason::Symlink,
                    });
                    continue;
                }
                rels.insert(rel);
            }
        }
        let states = self.all_states()?;
        let mut index = StoreIndex::build(&states);
        for (rel, _) in states {
            rels.insert(rel);
        }
        for rel in rels {
            let result = self.reconcile_path(&rel, &filter, None, &mut index, &mut report);
            tolerate_io(result, &rel, &mut report)?;
            self.run_reconcile_hook();
        }
        Ok(report)
    }

    /// Path states the peer with version vector `peer` has not merged.
    pub fn delta_for(&self, peer: &VersionVector) -> Result<Vec<PathUpdate>> {
        Ok(self
            .store
            .all()?
            .into_iter()
            .filter(|(_, s)| !peer.covers(&s.seen))
            .map(|(path, s)| PathUpdate {
                path,
                versions: s.versions,
                seen: s.seen,
            })
            .collect())
    }

    /// Join updates received from a peer, fetching content from `source`.
    ///
    /// Every update is joined before any file is written, so that the joined states go
    /// to the store in one transaction and what the files then hold in another, rather
    /// than several transactions per path.
    pub fn apply(&mut self, updates: &[PathUpdate], source: &dyn ContentSource) -> Result<Report> {
        if let Some(reason) = self.pause_reason()? {
            return Err(Error::Paused(reason));
        }
        let result = self.apply_updates(updates, source);
        self.finish(result)
    }

    fn apply_updates(
        &mut self,
        updates: &[PathUpdate],
        source: &dyn ContentSource,
    ) -> Result<Report> {
        let filter = self.filter()?;
        let mut report = Report::default();
        let mut index = StoreIndex::build(&self.all_states()?);
        let now = self.clock.now_ms();
        let mut joined = BTreeSet::new();
        for update in updates {
            // A counter of 0 is not a dot any replica assigns (`next_entry` increments
            // first), and it has no predecessor for `disk_seen` to pin a loser at.
            let well_formed = paths::is_valid_rel(&update.path)
                && !update.versions.is_empty()
                && update
                    .versions
                    .iter()
                    .all(|v| v.path == update.path && v.dot.counter > 0);
            if !well_formed {
                tracing::warn!(path = %update.path, "ignoring a malformed path update");
                continue;
            }
            for version in &update.versions {
                self.meta.hlc = self.meta.hlc.observe(version.hlc, now);
            }
            // Report a per-path I/O error and still integrate the update: the
            // on-disk check in `settle` is what keeps this from overwriting an
            // unrecorded local edit, and every delivered update must still be
            // joined into the store for `commit_session` to be correct.
            let reconciled =
                self.reconcile_path(&update.path, &filter, Some(source), &mut index, &mut report);
            tolerate_io(reconciled, &update.path, &mut report)?;
            if self
                .join(update.clone(), None, &mut index, &mut report)?
                .is_some()
            {
                joined.insert(update.path.clone());
            }
        }
        for rel in joined {
            let Some(state) = self.get_state(&rel)? else {
                continue;
            };
            let settled = self
                .ensure_conflict_copies(
                    &rel,
                    &state,
                    &filter,
                    Some(source),
                    &mut index,
                    &mut report,
                )
                .and_then(|()| {
                    self.settle(&rel, state, &filter, Some(source), &mut index, &mut report)
                });
            tolerate_io(settled, &rel, &mut report)?;
        }
        Ok(report)
    }

    /// Declare that every update of a session with `peer` has been applied.
    pub fn commit_session(&mut self, peer: &VersionVector) -> Result<()> {
        self.meta.vv.merge(peer);
        if self.pending.is_empty() {
            self.store.commit(&self.meta, &[])
        } else {
            self.flush()
        }
    }

    /// Retry writes that were waiting for content.
    pub fn fetch_missing(&mut self, source: &dyn ContentSource) -> Result<Report> {
        if let Some(reason) = self.pause_reason()? {
            return Err(Error::Paused(reason));
        }
        let result = self.fetch_missing_states(source);
        self.finish(result)
    }

    fn fetch_missing_states(&mut self, source: &dyn ContentSource) -> Result<Report> {
        let filter = self.filter()?;
        let mut report = Report::default();
        let states = self.all_states()?;
        let mut index = StoreIndex::build(&states);
        for (rel, _) in states {
            let result = self.reconcile_path(&rel, &filter, Some(source), &mut index, &mut report);
            tolerate_io(result, &rel, &mut report)?;
        }
        Ok(report)
    }

    /// Content this replica wants written and does not have: the winner of a path whose
    /// file does not hold it yet, and each loser still waiting for its conflict copy.
    ///
    /// A session asks its peer for these whatever the peer sends: committing a session
    /// takes in the peer's version vector even when some content never arrived, so no
    /// later delta carries those paths again. Content the replica would not write anyway
    /// — an ignored or unrepresentable path, a case collision, a file over the size limit
    /// — is left out.
    pub fn unavailable(&self) -> Result<Vec<ContentHash>> {
        let filter = self.filter()?;
        let states = self.all_states()?;
        let index = StoreIndex::build(&states);
        let max = self.config.max_file_size;
        let mut wanted: BTreeSet<ContentHash> = BTreeSet::new();
        for (rel, state) in &states {
            let winner = state.winner();
            if let Content::File { hash, .. } = winner.content
                && state.disk.hash != Some(hash)
                && self.skip_reason(rel, winner, &filter, &index)?.is_none()
            {
                wanted.insert(hash);
            }
            for loser in state
                .versions
                .iter()
                .filter(|v| v.dot != winner.dot && merge::needs_copy(v, winner))
            {
                let Content::File { hash, len } = loser.content else {
                    continue;
                };
                let copy_rel = merge::conflict_path(rel, &loser.dot);
                if state.disk.hash != Some(hash)
                    && len <= max
                    && filter.allows(&copy_rel, false)
                    && paths::representable(&copy_rel)
                    && self.get_state(&copy_rel)?.is_none()
                {
                    wanted.insert(hash);
                }
            }
        }
        let mut missing = Vec::new();
        for hash in wanted {
            if self.content_path_with(&hash, &index)?.is_none() {
                missing.push(hash);
            }
        }
        Ok(missing)
    }

    /// Remove staged content nothing needs any more, returning how many files went.
    ///
    /// Staged content is kept while some stored version refers to it and no workspace
    /// file holds it: the replica may still write it, or be asked for it. Placing content
    /// moves it out of staging, so what this removes is content staged and never placed —
    /// a version superseded first, a second copy from another session, inline content of
    /// a page that never came. A workspace file counts as holding the content only when
    /// its recorded stamp still matches and is not racy; nothing is re-hashed here.
    ///
    /// Files younger than [`STAGING_GRACE`] are kept whatever they hold: a session stages
    /// inline content before the page that refers to it arrives, and runs alongside this.
    pub fn collect_staging(&self) -> Result<usize> {
        let staged = staging::finished(&self.config.staging_dir)?;
        if staged.is_empty() {
            return Ok(0);
        }
        let states = self.all_states()?;
        let referenced: BTreeSet<ContentHash> = states
            .iter()
            .flat_map(|(_, state)| &state.versions)
            .filter_map(|v| v.content.hash())
            .collect();
        let index = StoreIndex::build(&states);
        let now = SystemTime::now();
        let mut removed = 0;
        for (hash, path, meta) in staged {
            let young = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_none_or(|age| age < STAGING_GRACE);
            if young {
                continue;
            }
            if referenced.contains(&hash) && !self.held_in_workspace(&hash, &index)? {
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::debug!(path = %path.display(), "could not remove staged content: {e}")
                }
            }
        }
        Ok(removed)
    }

    /// [`Self::collect_staging`], logging rather than returning a failure: collecting is
    /// housekeeping, and must not fail the open or scan it follows.
    fn collect_staging_quietly(&self) {
        match self.collect_staging() {
            Ok(0) => {}
            Ok(n) => tracing::debug!(removed = n, "collected staged content"),
            Err(e) => tracing::warn!("could not collect staged content: {e}"),
        }
    }

    /// Whether a workspace file holds `hash`, judged by its recorded stamp alone.
    fn held_in_workspace(&self, hash: &ContentHash, index: &StoreIndex) -> Result<bool> {
        for rel in index.paths_with(hash) {
            let Some(state) = self.get_state(rel)? else {
                continue;
            };
            let disk = &state.disk;
            if !is_racy(disk)
                && fs::metadata(paths::to_native(&self.config.root, rel))
                    .is_ok_and(|meta| stamp_of(&meta) == (disk.mtime_ns, disk.len))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A [`Stager`] over this replica's staging directory and size limit.
    pub fn stager(&self) -> Stager {
        Stager::new(self.config.staging_dir.clone(), self.config.max_file_size)
    }

    /// A file holding the content `hash`, and its length: a synced file on disk, else
    /// finished staged content. A file whose length and mtime still match what was
    /// recorded (and are not racy) is trusted; any other is re-hashed first.
    pub fn content_path(&self, hash: &ContentHash) -> Result<Option<(PathBuf, u64)>> {
        self.content_path_with(hash, &StoreIndex::build(&self.all_states()?))
    }

    /// [`Self::content_path`] for each of `hashes`, building the store index once for
    /// all of them rather than once per hash. A hash no file or staged content holds is
    /// left out of the map.
    pub fn content_paths(
        &self,
        hashes: &[ContentHash],
    ) -> Result<HashMap<ContentHash, (PathBuf, u64)>> {
        let index = StoreIndex::build(&self.all_states()?);
        let mut found = HashMap::new();
        for hash in hashes {
            if found.contains_key(hash) {
                continue;
            }
            if let Some(hit) = self.content_path_with(hash, &index)? {
                found.insert(*hash, hit);
            }
        }
        Ok(found)
    }

    /// [`Self::content_path`] against an index the caller already holds.
    fn content_path_with(
        &self,
        hash: &ContentHash,
        index: &StoreIndex,
    ) -> Result<Option<(PathBuf, u64)>> {
        for rel in index.paths_with(hash) {
            let Some(state) = self.get_state(rel)? else {
                continue;
            };
            let path = paths::to_native(&self.config.root, rel);
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let (mtime_ns, len) = stamp_of(&meta);
            let disk = &state.disk;
            let trusted = disk.hash.as_ref() == Some(hash)
                && (mtime_ns, len) == (disk.mtime_ns, disk.len)
                && !is_racy(disk);
            if trusted || ContentHash::of_file(&path).is_ok_and(|h| h == *hash) {
                return Ok(Some((path, len)));
            }
        }
        if let Some(path) = self.stager().staged(hash)
            && let Ok(meta) = fs::metadata(&path)
        {
            return Ok(Some((path, meta.len())));
        }
        Ok(None)
    }

    fn next_entry(&mut self, rel: &str, content: Content, context: VersionVector) -> Entry {
        self.meta.counter += 1;
        self.meta.hlc = self.meta.hlc.tick(self.clock.now_ms());
        let dot = Dot {
            replica: self.meta.replica_id,
            counter: self.meta.counter,
        };
        self.meta.vv.add_dot(&dot);
        Entry {
            path: rel.to_owned(),
            content,
            hlc: self.meta.hlc,
            dot,
            context,
            author: self.config.device_id,
        }
    }

    /// Compare one path's file with its state: record an edit, or settle the state.
    /// A file holding a version the store already knows is settled, not recorded: a
    /// watcher-driven scan of content a live session just materialized must not re-record it
    /// as a local edit (issue #157).
    fn reconcile_path(
        &mut self,
        rel: &str,
        filter: &SyncFilter,
        source: Option<&dyn ContentSource>,
        index: &mut StoreIndex,
        report: &mut Report,
    ) -> Result<()> {
        if !filter.allows(rel, false) || !paths::representable(rel) {
            return Ok(());
        }
        let abs = paths::to_native(&self.config.root, rel);
        let state = self.get_state(rel)?;
        let disk = state.as_ref().map(|s| s.disk.clone()).unwrap_or_default();
        // On a case-insensitive filesystem, `a.txt` would find the file of `A.txt`.
        if disk.hash.is_none() && index.case_twin(rel).is_some() {
            if let Some(state) = state {
                self.settle(rel, state, filter, source, index, report)?;
            }
            return Ok(());
        }
        let fs_meta = match fs::symlink_metadata(&abs) {
            Ok(m) => Some(m),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let mut stamp = (0, 0);
        let file_hash = match &fs_meta {
            Some(m) if m.file_type().is_symlink() => {
                report.skipped.push(Skipped {
                    path: rel.to_owned(),
                    reason: SkipReason::Symlink,
                });
                return Ok(());
            }
            Some(m) if m.is_file() => {
                stamp = stamp_of(m);
                if disk.hash.is_some() && stamp == (disk.mtime_ns, disk.len) && !is_racy(&disk) {
                    disk.hash
                } else if m.len() > self.config.max_file_size {
                    report.skipped.push(Skipped {
                        path: rel.to_owned(),
                        reason: SkipReason::TooLarge {
                            len: m.len(),
                            max: self.config.max_file_size,
                        },
                    });
                    return Ok(());
                } else {
                    Some(ContentHash::of_file(&abs)?)
                }
            }
            _ => None,
        };

        if file_hash == disk.hash {
            if let Some(mut state) = state {
                if file_hash.is_some() && (stamp != (disk.mtime_ns, disk.len) || is_racy(&disk)) {
                    state.disk.mtime_ns = stamp.0;
                    state.disk.len = stamp.1;
                    state.disk.checked_ns = now_ns();
                    self.commit_state(rel, &state, index)?;
                }
                self.ensure_conflict_copies(rel, &state, filter, source, index, report)?;
                self.settle(rel, state, filter, source, index, report)?;
            }
            return Ok(());
        }

        // A watcher-driven scan can fire after a live session has already committed a peer's
        // version to the store and written its bytes to disk. The file then holds content the
        // store already knows, but `disk.hash` still names the file the commit replaced, so
        // `file_hash != disk.hash`. Recording it here would invent a fresh own dot, sibling to
        // the version just received: a spurious conflict copy, and a delete context polluted
        // enough to stop a later delete from propagating (issue #157). Content the store
        // already knows is a materialization to settle, not a local edit to record.
        //
        // Matched against the *winner* specifically, not any sibling (issue #161). A path in
        // conflict keeps every loser's bytes alive too — as a conflict copy next to it — so a
        // local write can land on a loser's content without having materialized anything: a
        // deliberate edit that merely coincides with a version already present elsewhere in
        // the conflict set, not a re-appearance of what this scan's own disk state already
        // reflects. The winner is the one version this branch's premise actually describes
        // (disk already *is* this content, only the bookkeeping is stale), and it is also
        // `settle`'s own criterion for "nothing to do" once `disk.hash` is refreshed below —
        // matching any sibling let a write that changes which content wins slip through
        // silently, destroying the winner's only copy (the file) with nothing recording that
        // anything superseded it, and no conflict copy to fall back on (conflict copies exist
        // for losers, never for the winner).
        //
        // A missing file whose winner is a tombstone is the same case: the delete was
        // carried out, and a crash came before it was recorded.
        if state
            .as_ref()
            .is_some_and(|s| s.winner().content.hash() == file_hash)
        {
            if let Some(mut state) = state {
                // The on-disk file now holds `file_hash`, but the stored `disk` still
                // describes the file the commit replaced; refresh it so `settle` sees the
                // disk as already matching the known version instead of re-materializing it.
                if state.disk.hash != file_hash
                    || stamp != (disk.mtime_ns, disk.len)
                    || is_racy(&disk)
                {
                    state.disk.hash = file_hash;
                    state.disk.mtime_ns = stamp.0;
                    state.disk.len = stamp.1;
                    state.disk.checked_ns = now_ns();
                    self.commit_state(rel, &state, index)?;
                }
                self.ensure_conflict_copies(rel, &state, filter, source, index, report)?;
                self.settle(rel, state, filter, source, index, report)?;
            }
            return Ok(());
        }

        let content = match file_hash {
            Some(hash) => Content::File { hash, len: stamp.1 },
            None => Content::Tombstone,
        };
        if state.is_none() && content.is_tombstone() {
            return Ok(());
        }
        // A root that disappears *during* a scan looks exactly like a bulk delete: the
        // entry guard already passed, so every remaining path would be tombstoned and
        // the next push would delete them everywhere. Re-check before recording a
        // delete for a file this replica had materialized. `Error::Paused` is not
        // `Error::Io`, so `tolerate_io` propagates it and the scan stops here.
        if content.is_tombstone()
            && disk.hash.is_some()
            && let Some(reason) = self.pause_reason()?
        {
            return Err(Error::Paused(reason));
        }
        let entry = self.next_entry(rel, content, disk.seen.clone());
        report.recorded.push(entry.clone());
        let mut seen = entry.context.clone();
        seen.add_dot(&entry.dot);
        let new_disk = DiskState {
            hash: file_hash,
            seen: seen.clone(),
            mtime_ns: stamp.0,
            len: stamp.1,
            checked_ns: now_ns(),
        };
        let update = PathUpdate {
            path: rel.to_owned(),
            versions: vec![entry],
            seen,
        };
        self.integrate(update, Some(new_disk), filter, source, index, report)
    }

    /// Join `update` into the stored state and commit, then settle the file.
    /// `local_disk` is set when the update is a local write already on disk.
    fn integrate(
        &mut self,
        update: PathUpdate,
        local_disk: Option<DiskState>,
        filter: &SyncFilter,
        source: Option<&dyn ContentSource>,
        index: &mut StoreIndex,
        report: &mut Report,
    ) -> Result<()> {
        let rel = update.path.clone();
        let Some(state) = self.join(update, local_disk, index, report)? else {
            return Ok(());
        };
        // Copies first: settling may overwrite the loser's bytes on disk.
        self.ensure_conflict_copies(&rel, &state, filter, source, index, report)?;
        self.settle(&rel, state, filter, source, index, report)
    }

    /// Join `update` into the stored state and commit it, leaving the file as it is.
    /// The joined state, or `None` when the update brought nothing new.
    fn join(
        &mut self,
        update: PathUpdate,
        local_disk: Option<DiskState>,
        index: &mut StoreIndex,
        report: &mut Report,
    ) -> Result<Option<PathState>> {
        let rel = update.path;
        let old = self.get_state(&rel)?;
        let joined = merge::join(
            old.as_ref().map(|s| (s.versions.as_slice(), &s.seen)),
            &update.versions,
            &update.seen,
        );
        let Some((versions, seen)) = joined else {
            return Ok(None);
        };
        let disk = match local_disk {
            Some(d) => d,
            None => old.map(|s| s.disk).unwrap_or_default(),
        };
        let state = PathState {
            versions,
            seen,
            disk,
        };
        self.commit_state(&rel, &state, index)?;
        self.pending_versions = true;
        report.changed += 1;
        Ok(Some(state))
    }

    /// Write a conflict copy for every loser that needs one and has never had one.
    fn ensure_conflict_copies(
        &mut self,
        rel: &str,
        state: &PathState,
        filter: &SyncFilter,
        source: Option<&dyn ContentSource>,
        index: &mut StoreIndex,
        report: &mut Report,
    ) -> Result<()> {
        if state.versions.len() < 2 {
            return Ok(());
        }
        let winner = state.winner().clone();
        let losers: Vec<Entry> = state
            .versions
            .iter()
            .filter(|v| v.dot != winner.dot && merge::needs_copy(v, &winner))
            .cloned()
            .collect();
        for loser in losers {
            let Content::File { hash, .. } = loser.content else {
                continue;
            };
            let copy_rel = merge::conflict_path(rel, &loser.dot);
            if !filter.allows(&copy_rel, false) {
                report.skipped.push(Skipped {
                    path: copy_rel,
                    reason: SkipReason::Ignored,
                });
                continue;
            }
            if !paths::representable(&copy_rel) {
                report.skipped.push(Skipped {
                    path: copy_rel,
                    reason: SkipReason::Unrepresentable,
                });
                continue;
            }
            if self.get_state(&copy_rel)?.is_some() {
                continue;
            }
            let copy_abs = paths::to_native(&self.config.root, &copy_rel);
            // Only the loser's own bytes count as the copy already being there. Anything
            // else at the copy path is an unrelated file, and recording it as the copy
            // (which the reconcile below does) would lose the loser silently.
            let holds_the_loser = match fs::symlink_metadata(&copy_abs) {
                Ok(m) if m.is_file() && ContentHash::of_file(&copy_abs)? == hash => true,
                // A regular file with different bytes, a directory, or a symlink: not
                // the loser's bytes, and not something to write over. The loser stays
                // out of `disk.seen`, so it is not lost.
                Ok(_) => {
                    report.skipped.push(Skipped {
                        path: copy_rel,
                        reason: SkipReason::Occupied,
                    });
                    continue;
                }
                Err(e) if e.kind() == ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            if !holds_the_loser {
                // The loser may be exactly what is on disk at `rel` right now. A read
                // error here must propagate rather than be swallowed as unavailable
                // content: `settle` would otherwise overwrite the only local copy of
                // the loser's bytes.
                //
                // The file on disk is copied whatever its size: the limit may have
                // been lowered since it was recorded, and the copy is what keeps the
                // loser from being overwritten.
                let staged = if state.disk.hash == Some(hash) {
                    let unbounded = Stager::new(self.config.staging_dir.clone(), u64::MAX);
                    let from = paths::to_native(&self.config.root, rel);
                    Self::stage_with(&unbounded, &from, &hash)?
                } else {
                    let from = match self.content_path_with(&hash, index)? {
                        Some((path, _)) => Some(path),
                        None => source.and_then(|s| s.locate(&hash)),
                    };
                    match from {
                        Some(from) => self.stage_from(&from, &hash)?,
                        None => None,
                    }
                };
                let Some(staged) = staged else {
                    report.skipped.push(Skipped {
                        path: copy_rel,
                        reason: SkipReason::ContentUnavailable,
                    });
                    continue;
                };
                self.flush_versions()?;
                place(&staged, &copy_abs)?;
                report.conflicts.push(Conflict {
                    path: rel.to_owned(),
                    copy_path: copy_rel.clone(),
                });
            }
            // Record the copy (or a file someone already put there) as a local write.
            self.reconcile_path(&copy_rel, filter, source, index, report)?;
        }
        Ok(())
    }

    /// Make the file on disk hold the winner, then record what it reflects.
    fn settle(
        &mut self,
        rel: &str,
        mut state: PathState,
        filter: &SyncFilter,
        source: Option<&dyn ContentSource>,
        index: &mut StoreIndex,
        report: &mut Report,
    ) -> Result<()> {
        let winner = state.winner().clone();
        if winner.content.hash() == state.disk.hash {
            let seen = self.disk_seen(&state, filter)?;
            if seen != state.disk.seen {
                state.disk.seen = seen;
                self.commit_state(rel, &state, index)?;
            }
            return Ok(());
        }
        if let Some(reason) = self.skip_reason(rel, &winner, filter, index)? {
            report.skipped.push(Skipped {
                path: rel.to_owned(),
                reason,
            });
            return Ok(());
        }
        if self.occupied(rel, &state.disk)? {
            report.skipped.push(Skipped {
                path: rel.to_owned(),
                reason: SkipReason::Occupied,
            });
            return Ok(());
        }
        let abs = paths::to_native(&self.config.root, rel);
        match winner.content {
            Content::Tombstone => {
                self.flush_versions()?;
                self.fault_check()?;
                match fs::remove_file(&abs) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
                self.crash_check()?;
                state.disk = DiskState::default();
            }
            Content::File { hash, .. } => {
                let Some(staged) = self.stage(&hash, source, index)? else {
                    report.skipped.push(Skipped {
                        path: rel.to_owned(),
                        reason: SkipReason::ContentUnavailable,
                    });
                    return Ok(());
                };
                self.flush_versions()?;
                self.fault_check()?;
                place(&staged, &abs)?;
                self.crash_check()?;
                let (mtime_ns, len) = stamp_of(&fs::metadata(&abs)?);
                state.disk = DiskState {
                    hash: Some(hash),
                    seen: VersionVector::new(),
                    mtime_ns,
                    len,
                    checked_ns: now_ns(),
                };
            }
        }
        state.disk.seen = self.disk_seen(&state, filter)?;
        self.commit_state(rel, &state, index)
    }

    /// Commit one path state and keep `index` in step with it. The state is pending
    /// until [`Self::flush`], which this calls once enough states are pending.
    fn commit_state(&mut self, rel: &str, state: &PathState, index: &mut StoreIndex) -> Result<()> {
        self.pending.insert(rel.to_owned(), state.clone());
        index.record(rel, state.disk.hash);
        if self.pending.len() >= MAX_PENDING {
            self.flush()?;
        }
        Ok(())
    }

    /// Versions the file on disk reflects once the winner is written: everything
    /// merged, except losers still waiting for a conflict copy — a local edit must
    /// not supersede a version whose bytes were never preserved. A loser whose copy
    /// path the filter rejects, or that the local OS cannot represent, is never
    /// excluded this way: no copy will ever be attempted for it, so pinning it here
    /// would keep it out of `seen` forever.
    fn disk_seen(&self, state: &PathState, filter: &SyncFilter) -> Result<VersionVector> {
        let winner = state.winner();
        let mut seen = state.seen.clone();
        for loser in state
            .versions
            .iter()
            .filter(|v| v.dot != winner.dot && merge::needs_copy(v, winner))
        {
            let copy_rel = merge::conflict_path(&loser.path, &loser.dot);
            if !filter.allows(&copy_rel, false) || !paths::representable(&copy_rel) {
                continue;
            }
            if self.get_state(&copy_rel)?.is_none() {
                let slot = seen.0.entry(loser.dot.replica).or_insert(0);
                *slot = (*slot).min(loser.dot.counter.saturating_sub(1));
            }
        }
        Ok(seen)
    }

    fn skip_reason(
        &self,
        rel: &str,
        winner: &Entry,
        filter: &SyncFilter,
        index: &StoreIndex,
    ) -> Result<Option<SkipReason>> {
        if !filter.allows(rel, false) {
            return Ok(Some(SkipReason::Ignored));
        }
        if !paths::representable(rel) {
            return Ok(Some(SkipReason::Unrepresentable));
        }
        if let Content::File { len, .. } = winner.content
            && len > self.config.max_file_size
        {
            return Ok(Some(SkipReason::TooLarge {
                len,
                max: self.config.max_file_size,
            }));
        }
        if !winner.content.is_tombstone()
            && let Some(other) = index.case_twin(rel)
        {
            return Ok(Some(SkipReason::CaseCollision { other }));
        }
        Ok(None)
    }

    /// Whether the path on disk at `rel` holds something this replica has not
    /// recorded: a symlink, a directory, a file over the size cap, or a file whose
    /// content differs from `disk.hash`. A missing file, or a file matching
    /// `disk.hash`, is not occupied.
    fn occupied(&self, rel: &str, disk: &DiskState) -> Result<bool> {
        let abs = paths::to_native(&self.config.root, rel);
        let meta = match fs::symlink_metadata(&abs) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        if !meta.is_file() {
            // A symlink or a directory.
            return Ok(true);
        }
        if meta.len() > self.config.max_file_size {
            return Ok(true);
        }
        let stamp = stamp_of(&meta);
        let hash = if disk.hash.is_some() && stamp == (disk.mtime_ns, disk.len) && !is_racy(disk) {
            disk.hash
        } else {
            Some(ContentHash::of_file(&abs)?)
        };
        Ok(hash != disk.hash)
    }

    /// A verified copy of `hash` in the staging directory, if one can be had.
    fn stage(
        &self,
        hash: &ContentHash,
        source: Option<&dyn ContentSource>,
        index: &StoreIndex,
    ) -> Result<Option<PathBuf>> {
        // Anything staged under its hash was verified when it was finished.
        if let Some(staged) = self.stager().staged(hash) {
            return Ok(Some(staged));
        }
        let from = match self.content_path_with(hash, index)? {
            Some((path, _)) => path,
            None => match source.and_then(|s| s.locate(hash)) {
                Some(path) => path,
                None => return Ok(None),
            },
        };
        self.stage_from(&from, hash)
    }

    /// Copy the file `from` into the staging directory as the content `hash`, within
    /// the size limit. Content that is over the limit or does not hash to `hash` is
    /// logged and treated as unavailable; a failure to read `from` or write the copy
    /// is an error.
    fn stage_from(&self, from: &Path, hash: &ContentHash) -> Result<Option<PathBuf>> {
        Self::stage_with(&self.stager(), from, hash)
    }

    /// [`Self::stage_from`] through the given stager.
    fn stage_with(stager: &Stager, from: &Path, hash: &ContentHash) -> Result<Option<PathBuf>> {
        let mut file = fs::File::open(from)?;
        let len = file.metadata()?.len();
        let mut writer = match stager.writer(*hash, len) {
            Ok(writer) => writer,
            Err(Error::TooLarge { len, max }) => {
                tracing::warn!(%hash, len, max, "content is over the size limit");
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        // A file that grows after its length was read overruns the writer mid-copy.
        if let Err(e) = std::io::copy(&mut file, &mut writer) {
            return match e.get_ref().and_then(|i| i.downcast_ref::<Error>()) {
                Some(Error::TooLarge { .. } | Error::ContentMismatch { .. }) => {
                    tracing::warn!(%hash, "content changed while it was copied");
                    Ok(None)
                }
                _ => Err(e.into()),
            };
        }
        match writer.finish() {
            Ok(path) => Ok(Some(path)),
            Err(Error::ContentMismatch { .. }) => {
                tracing::warn!(%hash, "content did not match its hash");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

impl ContentSource for Replica {
    fn locate(&self, hash: &ContentHash) -> Option<PathBuf> {
        self.content_path(hash).ok().flatten().map(|(path, _)| path)
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Replica {
    /// Open with a fixed replica id (used only when the store is created).
    pub fn open_with_replica_id(
        config: ReplicaConfig,
        clock: Arc<dyn Clock>,
        id: ReplicaId,
    ) -> Result<Self> {
        Self::open_inner(config, clock, Some(id))
    }

    /// Fire `point` the next time it is reached, then clear it.
    pub fn inject_fault(&mut self, point: crate::testing::FaultPoint) {
        self.fault = Some(point);
    }

    /// Run `hook` once after the next path a scan reconciles, then clear it.
    pub fn inject_after_reconcile(&mut self, hook: Box<dyn FnOnce() + Send>) {
        self.reconcile_hook = Some(hook);
    }

    /// Everything that must agree across converged replicas, without timestamps of
    /// the local filesystem.
    pub fn logical_dump(&self) -> Result<serde_json::Value> {
        let mut path_states = serde_json::Map::new();
        for (rel, s) in self.all_states()? {
            path_states.insert(
                rel,
                serde_json::json!({
                    "versions": s.versions,
                    "seen": s.seen,
                    "disk_hash": s.disk.hash,
                    "disk_seen": s.disk.seen,
                }),
            );
        }
        let mut files = serde_json::Map::new();
        if self.config.root.is_dir() {
            for item in walkdir::WalkDir::new(&self.config.root).sort_by_file_name() {
                let item = item.map_err(|e| Error::Io(std::io::Error::other(e)))?;
                if !item.file_type().is_file() {
                    continue;
                }
                if let Some(rel) = paths::rel_from_native(&self.config.root, item.path()) {
                    files.insert(rel, serde_json::json!(ContentHash::of_file(item.path())?));
                }
            }
        }
        Ok(serde_json::json!({
            "replica_id": self.meta.replica_id,
            "vv": self.meta.vv,
            "paths": path_states,
            "files": files,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(body: &str) -> ContentHash {
        ContentHash::of_bytes(body.as_bytes())
    }

    #[test]
    fn the_index_follows_a_path_from_one_hash_to_another() {
        let mut index = StoreIndex::default();
        index.record("a.txt", Some(hash("one")));
        index.record("b.txt", Some(hash("one")));
        assert_eq!(
            index.paths_with(&hash("one")).collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"]
        );

        index.record("a.txt", Some(hash("two")));
        assert_eq!(
            index.paths_with(&hash("one")).collect::<Vec<_>>(),
            vec!["b.txt"],
            "the stale hash no longer points at a.txt"
        );
        assert_eq!(
            index.paths_with(&hash("two")).collect::<Vec<_>>(),
            vec!["a.txt"]
        );

        // A deleted file has no bytes on disk any more.
        index.record("a.txt", None);
        assert_eq!(index.paths_with(&hash("two")).count(), 0);
    }

    #[test]
    fn the_index_finds_a_materialized_case_twin() {
        let mut index = StoreIndex::default();
        index.record("A.txt", Some(hash("upper")));
        index.record("a.txt", None);
        let twin = index.case_twin("a.txt");
        if paths::CASE_INSENSITIVE_FS {
            assert_eq!(twin.as_deref(), Some("A.txt"));
            assert_eq!(index.case_twin("A.txt"), None, "a.txt is not on disk");
            assert_eq!(index.case_twin("other.txt"), None);
        } else {
            assert_eq!(twin, None, "no case twins on a case-sensitive filesystem");
        }
    }
}
