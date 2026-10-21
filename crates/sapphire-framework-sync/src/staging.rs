//! Verified staging of incoming content.
//!
//! Incoming bytes are streamed into a partial file of their own, checked against the
//! announced length and hash, and only then renamed to `<hex>`. A file named by a bare
//! hash in the staging directory is therefore always complete and correct.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use crate::hash::{ContentHash, ContentHasher};

const PARTIAL_SUFFIX: &str = ".partial";

/// Tells apart the partial files of writers in this process; the pid tells apart
/// processes.
static NEXT_PARTIAL: AtomicU64 = AtomicU64::new(0);

/// Create a partial file for `hash` in `dir` that no other writer has, as
/// `<hex>.<pid>-<n>.partial`.
fn create_partial(dir: &Path, hash: &ContentHash) -> io::Result<(File, PathBuf)> {
    loop {
        let n = NEXT_PARTIAL.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(
            "{}.{}-{n}{PARTIAL_SUFFIX}",
            hash.to_hex(),
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            // Left by an earlier process that had the same pid.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// A staging directory that only ever holds verified content under its hash.
#[derive(Clone, Debug)]
pub struct Stager {
    dir: PathBuf,
    max: u64,
}

impl Stager {
    /// A stager over `dir` that refuses content longer than `max` bytes.
    pub fn new(dir: PathBuf, max: u64) -> Self {
        Self { dir, max }
    }

    /// Start receiving the content `hash` of `len` bytes into a partial file of the
    /// writer's own, so writers for the same hash (two sessions, or a session and a
    /// scan's conflict copy) never share one. Fails with [`Error::TooLarge`] before
    /// touching the disk when `len` is over the limit.
    pub fn writer(&self, hash: ContentHash, len: u64) -> Result<StagingWriter> {
        if len > self.max {
            return Err(Error::TooLarge { len, max: self.max });
        }
        fs::create_dir_all(&self.dir)?;
        let (file, partial) = create_partial(&self.dir, &hash)?;
        Ok(StagingWriter {
            file,
            hasher: ContentHasher::new(),
            written: 0,
            len,
            hash,
            partial,
            dest: self.dir.join(hash.to_hex()),
            finished: false,
        })
    }

    /// The finished content for `hash`, if it is staged. Only existence is checked:
    /// anything at `<hex>` was verified before it was renamed there.
    pub fn staged(&self, hash: &ContentHash) -> Option<PathBuf> {
        let path = self.dir.join(hash.to_hex());
        path.is_file().then_some(path)
    }
}

/// Receives one blob into its own partial file, verifying it on
/// [`finish`](Self::finish). Dropping it unfinished removes that file.
pub struct StagingWriter {
    file: File,
    hasher: ContentHasher,
    written: u64,
    len: u64,
    hash: ContentHash,
    partial: PathBuf,
    dest: PathBuf,
    finished: bool,
}

impl StagingWriter {
    /// Append the next piece. Fails with [`Error::TooLarge`] if the total would pass
    /// the announced length.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let total = self.written.saturating_add(bytes.len() as u64);
        if total > self.len {
            return Err(Error::TooLarge {
                len: total,
                max: self.len,
            });
        }
        self.file.write_all(bytes)?;
        self.hasher.update(bytes);
        self.written = total;
        Ok(())
    }

    /// Check the length and hash, then move the file to `<hex>` and return its path.
    /// Anything other than the announced content is [`Error::ContentMismatch`]. If
    /// another writer staged the same hash meanwhile, the rename replaces its file
    /// with identical content.
    ///
    /// The bytes are on disk before the rename, so a crash cannot leave a `<hex>` that
    /// holds less than verified content. The rename itself may be lost to one; that
    /// leaves only a `.partial`, which [`clear_partials`] removes, and the content is
    /// asked for again.
    pub fn finish(mut self) -> Result<PathBuf> {
        if self.written != self.len || self.hasher.clone().finish() != self.hash {
            return Err(Error::ContentMismatch {
                expected: self.hash,
            });
        }
        self.file.flush()?;
        self.file.sync_all()?;
        fs::rename(&self.partial, &self.dest)?;
        self.finished = true;
        Ok(self.dest.clone())
    }
}

impl Write for StagingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        StagingWriter::write(self, buf).map_err(|e| match e {
            Error::Io(e) => e,
            other => io::Error::other(other),
        })?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Drop for StagingWriter {
    fn drop(&mut self) {
        if !self.finished {
            let _ = fs::remove_file(&self.partial);
        }
    }
}

/// Remove leftover `*.partial` files from `dir`, keeping finished content. A missing
/// `dir` is fine.
pub(crate) fn clear_partials(dir: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "partial") {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// The finished content in `dir`: each file named by a bare hash, with its metadata.
/// Partial files and anything else are left out. A missing `dir` holds nothing.
pub(crate) fn finished(dir: &Path) -> io::Result<Vec<(ContentHash, PathBuf, fs::Metadata)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(hash) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<ContentHash>().ok())
        else {
            continue;
        };
        let meta = entry.metadata()?;
        if meta.is_file() {
            found.push((hash, entry.path(), meta));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::hash::{ContentHash, ContentHasher};

    fn dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("staging");
        (tmp, dir)
    }

    fn entries(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
    }

    #[test]
    fn hasher_matches_of_bytes_across_split_updates() {
        let mut h = ContentHasher::new();
        h.update(b"ab");
        h.update(b"c");
        assert_eq!(h.finish(), ContentHash::of_bytes(b"abc"));
    }

    #[test]
    fn finished_content_lands_at_its_hash() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"hello");
        let mut w = Stager::new(dir.clone(), 100).writer(hash, 5).unwrap();
        w.write(b"hel").unwrap();
        w.write(b"lo").unwrap();
        let path = w.finish().unwrap();
        assert_eq!(path, dir.join(hash.to_hex()));
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(entries(&dir), 1, "no .partial left");
    }

    #[test]
    fn a_wrong_hash_is_refused_and_leaves_nothing() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"hello");
        let mut w = Stager::new(dir.clone(), 100).writer(hash, 5).unwrap();
        w.write(b"world").unwrap();
        assert!(matches!(
            w.finish(),
            Err(Error::ContentMismatch { expected }) if expected == hash
        ));
        assert_eq!(entries(&dir), 0);
    }

    #[test]
    fn a_short_write_is_refused_and_leaves_nothing() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"hello");
        let mut w = Stager::new(dir.clone(), 100).writer(hash, 5).unwrap();
        w.write(b"hel").unwrap();
        assert!(matches!(w.finish(), Err(Error::ContentMismatch { .. })));
        assert_eq!(entries(&dir), 0);
    }

    #[test]
    fn writing_past_len_is_refused() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"abc");
        let mut w = Stager::new(dir, 100).writer(hash, 3).unwrap();
        assert!(matches!(
            w.write(b"abcd"),
            Err(Error::TooLarge { len: 4, max: 3 })
        ));
    }

    #[test]
    fn a_len_over_the_max_is_refused_before_any_file() {
        let (_t, dir) = dir();
        let stager = Stager::new(dir.clone(), 10);
        let err = stager.writer(ContentHash::of_bytes(b"x"), 11).err();
        assert!(matches!(err, Some(Error::TooLarge { len: 11, max: 10 })));
        assert_eq!(entries(&dir), 0);
    }

    #[test]
    fn dropping_unfinished_removes_the_partial() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"hello");
        let mut w = Stager::new(dir.clone(), 100).writer(hash, 5).unwrap();
        w.write(b"he").unwrap();
        assert_eq!(entries(&dir), 1);
        drop(w);
        assert_eq!(entries(&dir), 0);
    }

    #[test]
    fn a_zero_length_blob_finishes() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"");
        let w = Stager::new(dir.clone(), 100).writer(hash, 0).unwrap();
        let path = w.finish().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"");
    }

    #[test]
    fn staged_reports_only_finished_content() {
        let (_t, dir) = dir();
        let stager = Stager::new(dir, 100);
        let hash = ContentHash::of_bytes(b"hi");
        let mut w = stager.writer(hash, 2).unwrap();
        w.write(b"hi").unwrap();
        assert_eq!(stager.staged(&hash), None);
        let path = w.finish().unwrap();
        assert_eq!(stager.staged(&hash), Some(path));
    }

    #[test]
    fn a_vanished_staging_dir_fails_and_leaves_no_partial() {
        let (_t, dir) = dir();
        let hash = ContentHash::of_bytes(b"hello");
        let mut w = Stager::new(dir.clone(), 100).writer(hash, 5).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let result = w.write(b"hello").and_then(|()| w.finish().map(|_| ()));
        assert!(result.is_err());
        assert!(!dir.exists());
    }

    /// Two writers for one hash, as two sessions or a session and a scan's conflict
    /// copy can be. `a_first` picks which one finishes first.
    fn two_writers_for_one_hash(a_first: bool) {
        let (_t, dir) = dir();
        let stager = Stager::new(dir.clone(), 100);
        let hash = ContentHash::of_bytes(b"hello");
        let mut a = stager.writer(hash, 5).unwrap();
        a.write(b"hel").unwrap();
        let mut b = stager.writer(hash, 5).unwrap();
        b.write(b"h").unwrap();
        a.write(b"lo").unwrap();
        b.write(b"el").unwrap();
        // `a` is complete, `b` is behind; one finishes before the other has.
        let path = if a_first {
            let path = a.finish().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"hello");
            b.write(b"lo").unwrap();
            b.finish().unwrap()
        } else {
            b.write(b"lo").unwrap();
            let path = b.finish().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"hello");
            a.finish().unwrap()
        };
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(stager.staged(&hash), Some(path));
        assert_eq!(entries(&dir), 1, "no .partial left");
    }

    #[test]
    fn two_writers_for_one_hash_finishing_first_to_last() {
        two_writers_for_one_hash(true);
    }

    #[test]
    fn two_writers_for_one_hash_finishing_last_to_first() {
        two_writers_for_one_hash(false);
    }

    #[test]
    fn dropping_one_writer_leaves_the_other_alone() {
        let (_t, dir) = dir();
        let stager = Stager::new(dir.clone(), 100);
        let hash = ContentHash::of_bytes(b"hello");
        let mut a = stager.writer(hash, 5).unwrap();
        a.write(b"he").unwrap();
        let mut b = stager.writer(hash, 5).unwrap();
        b.write(b"h").unwrap();
        drop(b);
        a.write(b"llo").unwrap();
        let path = a.finish().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"hello");
        assert_eq!(entries(&dir), 1);
    }

    #[test]
    fn clear_partials_removes_the_partials_of_abandoned_writers() {
        let (_t, dir) = dir();
        let stager = Stager::new(dir.clone(), 100);
        let hash = ContentHash::of_bytes(b"hello");
        for _ in 0..2 {
            let mut w = stager.writer(hash, 5).unwrap();
            w.write(b"he").unwrap();
            // A crash: the writer never gets to clean up.
            std::mem::forget(w);
        }
        assert_eq!(entries(&dir), 2, "each writer has its own partial");
        clear_partials(&dir).unwrap();
        assert_eq!(entries(&dir), 0);
    }

    #[test]
    fn clear_partials_removes_only_partials() {
        let (_t, dir) = dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aa"), b"keep").unwrap();
        std::fs::write(dir.join("aa.partial"), b"drop").unwrap();
        clear_partials(&dir).unwrap();
        assert!(dir.join("aa").exists());
        assert!(!dir.join("aa.partial").exists());
        clear_partials(&dir.join("missing")).unwrap();
    }
}
