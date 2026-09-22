//! The bridge's log: a file any process on this host can read.
//!
//! The bridge installs a `tracing` layer that writes its own targets to
//! `<bridge dir>/logs/node.log`, on top of whatever the process already prints. One process
//! writes — the single-instance lock guarantees it — so the file is a continuous record
//! across restarts rather than an interleaving. The file rotates at [`LOG_MAX_BYTES`], and
//! [`LOG_KEEP`] rotated files are kept.
//!
//! `bridge log` reads the file back: [`tail`] prints its last lines, and [`follow`] keeps
//! printing as it grows, in the manner of `tail -f`.
//!
//! See `docs/superpowers/specs/2026-09-16-process-architecture-design.md` §5.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::dir::BridgeDir;
use crate::error::Result;

/// The log file's name, inside the bridge directory's log directory.
pub const LOG_FILE: &str = "node.log";

/// The current log file is rotated once it holds this many bytes.
pub const LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// How many rotated files are kept, in addition to the current one.
pub const LOG_KEEP: usize = 3;

/// The log filter a bridge applies when the environment asks for nothing specific.
///
/// The same default the bridge's console output uses, so `bridge run` and a one-shot
/// command agree on what is worth printing without `RUST_LOG` being set.
pub const DEFAULT_LOG_FILTER: &str = "sapphire_framework_bridge=info";

/// The target prefix whose events the log file keeps.
///
/// The bridge's own modules; anything else the process logs — iroh, its dependencies —
/// stays on the console only, so the file answers "what did the bridge do". A caller that
/// wants its line in the file too names this target.
pub const BRIDGE_TARGET: &str = "sapphire_framework_bridge";

/// How often a following `bridge log` looks for more lines.
const FOLLOW_POLL: Duration = Duration::from_millis(250);

/// Where the file layer's events go right now.
///
/// `tracing` installs a global subscriber once per process and never replaces it, but a
/// bridge may be installed twice in one process's lifetime — a restart inside a supervisor,
/// and the tests. So the subscriber is built once, around this wire, and every [`install`]
/// swaps which writer the wire feeds. `None` means no bridge holds the log, and file events
/// go nowhere while the console layer keeps printing.
type Wire = Arc<RwLock<Option<Arc<NonBlocking>>>>;

/// Where the file layer's events go right now.
static WIRE: OnceLock<Wire> = OnceLock::new();

/// The process's subscriber: the console as the bridge already prints it, plus the file
/// layer behind [`WIRE`].
///
/// Called by the binary before dispatching, so every subcommand prints what it always
/// printed. If a global subscriber got set first — an embedding application's choice — the
/// attempt is given up quietly and the file layer simply never hears an event; the wire
/// still exists, so [`install`] keeps working for a caller that wants a writer anyway.
pub fn install_console() {
    WIRE.get_or_init(build_subscriber);
}

/// Install the bridge's log writer for `dir`, rotating at [`LOG_MAX_BYTES`].
///
/// What installing means:
/// - the file `<bridge dir>/logs/node.log` receives every event of the bridge's own
///   targets, in addition to whatever the process prints;
/// - the file is opened for appending, so a restarted bridge continues the same record;
/// - events reach the file through a writer thread, and the returned [`LogGuard`] flushes
///   it when dropped. Keep the guard for as long as the bridge serves.
///
/// Installing a second time — a bridge restarting inside this process — re-routes the file
/// layer to the new writer; the subscriber itself is built once. The single-instance lock
/// must be held by the caller: the log's one-writer property is the lock's, not this
/// function's.
pub fn install(dir: &BridgeDir) -> Result<LogGuard> {
    install_with_limit(dir, LOG_MAX_BYTES)
}

/// As [`install`], rotating at `limit` bytes rather than [`LOG_MAX_BYTES`].
///
/// A file that passes the limit is renamed aside — `node.log.1`, `node.log.2` and so on —
/// keeping [`LOG_KEEP`] rotated files and starting a fresh current one. Rotation is
/// approximate: a file may overshoot `limit` by as much as one event.
pub fn install_with_limit(dir: &BridgeDir, limit: u64) -> Result<LogGuard> {
    // Build the subscriber once; every later install only re-routes the file layer. A
    // caller that never ran `install_console` gets the subscriber here.
    let wire = WIRE.get_or_init(build_subscriber);

    let rotating = Rotating::open(dir.log_dir().join(LOG_FILE), limit, LOG_KEEP);
    let (writer, worker) = tracing_appender::non_blocking(rotating);
    let writer = Arc::new(writer);

    *wire
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&writer));
    Ok(LogGuard {
        wire: Arc::clone(wire),
        writer,
        _worker: worker,
    })
}

/// Build the subscriber the bridge prints through: the console layer, and the file layer
/// routing through `wire`.
fn build_subscriber() -> Wire {
    let wire: Wire = Arc::new(RwLock::new(None));

    // The console layer is rebuilt here rather than left to the caller: this subscriber is
    // the one the process prints through, and a bridge run must not lose the console output
    // it would have had.
    let console = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG_FILTER.into()),
        );
    let file = tracing_subscriber::fmt::layer()
        // A file any process reads wants text, not terminal escapes.
        .with_ansi(false)
        .with_writer(FileLayer {
            wire: Arc::clone(&wire),
        })
        .with_filter(
            Targets::new().with_target(BRIDGE_TARGET, tracing::level_filters::LevelFilter::TRACE),
        );

    let _ = tracing_subscriber::registry()
        .with(console)
        .with(file)
        .try_init();
    wire
}

/// The file layer's writer: whatever [`install`] last routed through [`WIRE`].
#[derive(Clone, Debug)]
struct FileLayer {
    wire: Wire,
}

impl<'a> MakeWriter<'a> for FileLayer {
    type Writer = RoutedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        let current = self
            .wire
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        RoutedWriter {
            writer: current.map(|routed| (*routed).clone()),
        }
    }
}

/// A writer holding the log writer that was current when the event was made.
#[derive(Debug)]
struct RoutedWriter {
    writer: Option<NonBlocking>,
}

impl Write for RoutedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match &mut self.writer {
            Some(writer) => writer.write(buf),
            // No bridge holds the log: the event was never meant for a file.
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.writer {
            Some(writer) => writer.flush(),
            None => Ok(()),
        }
    }
}

/// Keeps the log's writer thread alive, and flushes it when dropped.
///
/// Dropping it unhooks the wire first — so later events no longer queue into a worker that
/// is about to stop — and then shuts the worker down, flushing everything queued. A guard
/// is meant to bracket a bridge's lifetime: a newer [`install`] re-routes the wire, and an
/// older guard's drop leaves that routing alone.
#[derive(Debug)]
pub struct LogGuard {
    wire: Wire,
    writer: Arc<NonBlocking>,
    _worker: WorkerGuard,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        let mut wire = self
            .wire
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A newer install may have re-routed the wire already; only unhook our own.
        if wire
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.writer))
        {
            *wire = None;
        }
    }
}

/// The last `lines` lines of the log, in order.
///
/// A log that is not there yet is an empty tail rather than an error: the bridge has never
/// run on this host, which is a normal thing for `bridge log` to report. Only the current
/// file is read — lines an earlier rotation moved into `node.log.1` are the rotated files'
/// business.
pub fn tail(dir: &BridgeDir, lines: usize) -> Result<Vec<String>> {
    let bytes = match std::fs::read(dir.log_dir().join(LOG_FILE)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(last_lines(&bytes, lines))
}

/// Print the log's tail, then keep printing as the file grows, until `stop` is true.
///
/// `tail -f`, spelled out: the last `lines` lines go out at once, then each poll prints what
/// appeared since. A file that shrank — rotated or truncated under us — is read from its
/// start again, so following a rotating log keeps printing rather than stalling. Returns
/// once `stop` is true, after anything that appeared has been printed; the CLI never passes
/// a `stop` that fires, and ends the way `tail -f` does, by being interrupted.
pub(crate) fn follow(
    dir: &BridgeDir,
    lines: usize,
    out: &mut impl Write,
    stop: impl Fn() -> bool,
) -> Result<()> {
    let path = dir.log_dir().join(LOG_FILE);
    // The tail first, then continue from where it ended: the offset is the length of what
    // was read, so nothing between the read and the follow is printed twice.
    let mut offset = match std::fs::read(&path) {
        Ok(bytes) => {
            for line in last_lines(&bytes, lines) {
                writeln!(out, "{line}")?;
            }
            out.flush()?;
            bytes.len() as u64
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.into()),
    };

    loop {
        print_growth(&path, &mut offset, out)?;
        // Checked only after the drain, so a stop set while lines were landing still
        // prints them rather than ending one poll early.
        if stop() {
            return Ok(());
        }
        std::thread::sleep(FOLLOW_POLL);
    }
}

/// Print the bytes that appeared past `offset`, and leave `offset` at the new end.
fn print_growth(path: &Path, offset: &mut u64, out: &mut impl Write) -> Result<()> {
    let len = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        // The file is being rotated, or was removed: the next poll finds whatever took its
        // place, and the record to follow starts over from its first byte.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            *offset = 0;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    // The file shrank: it was rotated or truncated. Whatever we had read is gone from this
    // name, and the record to follow starts over.
    if len < *offset {
        *offset = 0;
    }
    if len == *offset {
        return Ok(());
    }
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(*offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    *offset = len;
    out.write_all(&bytes)?;
    out.flush()?;
    Ok(())
}

/// The last `lines` lines of `bytes`, in order, without their newlines.
///
/// Bytes after the last newline are a line still being written and are reported: a tail
/// that swallows them would hide what the bridge is saying right now.
fn last_lines(bytes: &[u8], lines: usize) -> Vec<String> {
    if lines == 0 {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(bytes);
    let collected: Vec<&str> = text.lines().collect();
    let start = collected.len().saturating_sub(lines);
    collected[start..]
        .iter()
        .map(|line| (*line).to_owned())
        .collect()
}

/// A log file that rotates itself once it passes a size limit.
///
/// Only the bridge writes this file — the single-instance lock sees to it — so the writer
/// needs no locking against a second process, only against its own rotation. The count of
/// bytes written travels with the open file, so no write has to ask the filesystem how big
/// the file is. The writer thread of `tracing-appender` is this file's only caller.
struct Rotating {
    path: std::path::PathBuf,
    limit: u64,
    keep: usize,
    state: Mutex<Option<Open>>,
}

/// One open log file, and how much it already holds.
struct Open {
    file: std::fs::File,
    written: u64,
}

impl Rotating {
    /// Open (or create) the log at `path`, rotating at `limit` and keeping `keep` rotated
    /// files.
    fn open(path: std::path::PathBuf, limit: u64, keep: usize) -> Rotating {
        Rotating {
            path,
            limit,
            keep,
            state: Mutex::new(None),
        }
    }

    /// Retire the oldest rotated file, shift the rest up, and open a fresh current file.
    fn rotate(&self, state: &mut Option<Open>) -> std::io::Result<()> {
        // The open file's name is about to change under it: drop the handle first, shift
        // the rotated names up, and open fresh, so the next bytes land in the new file.
        *state = None;
        let rotated = |n: usize| self.path.with_extension(format!("log.{n}"));
        for n in (1..self.keep).rev() {
            rename_away(&rotated(n), &rotated(n + 1))?;
        }
        rename_away(&self.path, &rotated(1))?;
        *state = Some(Open {
            file: open_append(&self.path)?,
            written: 0,
        });
        Ok(())
    }
}

/// Rename `from` to `to`, ignoring a `from` that is not there.
fn rename_away(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Open `path` for appending, creating it if absent.
///
/// Appending is what makes a restart continue the same record: no truncate, no rewrite.
fn open_append(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
}

impl Write for Rotating {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut state = self.state.lock().expect("rotating log state");
        if state.is_none() {
            // The file may already hold a record from earlier runs; the first write of a
            // restart continues it, which is why the count starts at what is on disk.
            let written = std::fs::metadata(&self.path).map_or(0, |m| m.len());
            let file = open_append(&self.path)?;
            *state = Some(Open { file, written });
        }
        let full = state
            .as_ref()
            .is_some_and(|open| open.written + buf.len() as u64 > self.limit);
        if full {
            self.rotate(&mut state)?;
        }
        let open = state.as_mut().expect("rotation left no file open");
        open.file.write_all(buf)?;
        open.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut state = self.state.lock().expect("rotating log state");
        if let Some(open) = state.as_mut() {
            open.file.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::BridgeDir;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The logging tests install the process's one global subscriber, and `cargo test`
    /// runs a binary's tests in parallel threads: one lock takes them turns, so one
    /// test's events never land in another test's file.
    static INSTALL_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn installing_creates_the_log_file() {
        let _turn = INSTALL_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let guard = install(&dir).unwrap();
        tracing::info!(target: "sapphire_framework_bridge", "hello from the test");
        drop(guard);

        let text = std::fs::read_to_string(dir.log_dir().join(LOG_FILE)).unwrap();
        assert!(text.contains("hello from the test"), "{text}");
    }

    #[test]
    fn the_log_rotates_at_its_size_limit() {
        let _turn = INSTALL_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let guard = install_with_limit(&dir, 4096).unwrap();
        for n in 0..2000 {
            tracing::info!(target: "sapphire_framework_bridge", "line {n} padded {}", "x".repeat(64));
        }
        drop(guard);

        let files: Vec<String> = std::fs::read_dir(dir.log_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(files.len() > 1, "the log never rotated: {files:?}");
        assert!(files.len() <= LOG_KEEP + 1, "too many kept: {files:?}");
    }

    #[test]
    fn a_restart_continues_the_same_file() {
        let _turn = INSTALL_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();

        let guard = install(&dir).unwrap();
        tracing::info!(target: "sapphire_framework_bridge", "first run");
        drop(guard);

        let guard = install(&dir).unwrap();
        tracing::info!(target: "sapphire_framework_bridge", "second run");
        drop(guard);

        let text = std::fs::read_to_string(dir.log_dir().join(LOG_FILE)).unwrap();
        assert!(
            text.contains("first run") && text.contains("second run"),
            "{text}"
        );
    }

    #[test]
    fn reading_the_tail_of_a_missing_log_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        assert!(tail(&dir, 20).unwrap().is_empty());
    }

    #[test]
    fn the_tail_returns_the_last_lines_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        std::fs::create_dir_all(dir.log_dir()).unwrap();
        std::fs::write(
            dir.log_dir().join(LOG_FILE),
            (0..100).map(|n| format!("line {n}\n")).collect::<String>(),
        )
        .unwrap();

        let lines = tail(&dir, 3).unwrap();
        assert_eq!(lines, vec!["line 97", "line 98", "line 99"]);
    }

    #[test]
    fn following_prints_the_lines_that_arrive() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let log = dir.log_dir().join(LOG_FILE);
        std::fs::write(&log, "line 0\nline 1\nline 2\n").unwrap();

        // Append more lines a moment later, then raise the flag that ends the follow.
        let arrived = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&arrived);
        let path = log.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(b"line 3\nline 4\n").unwrap();
            flag.store(true, Ordering::SeqCst);
        });

        // A deadline keeps a failed append from hanging the test: the stop fires either
        // way, and the assertions below tell which.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let flag = Arc::clone(&arrived);
        let stop = move || flag.load(Ordering::SeqCst) || std::time::Instant::now() > deadline;

        let mut out: Vec<u8> = Vec::new();
        follow(&dir, 10, &mut out, stop).unwrap();
        writer.join().unwrap();

        let text = String::from_utf8(out).unwrap();
        for line in ["line 0", "line 1", "line 2", "line 3", "line 4"] {
            assert!(text.contains(line), "missing {line}: {text}");
        }
        let (first, last) = (text.find("line 0").unwrap(), text.find("line 4").unwrap());
        assert!(
            first < last,
            "the tail and the growth came out of order: {text}"
        );
    }
}
