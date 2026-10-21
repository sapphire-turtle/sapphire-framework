//! Content arriving from the peer, written straight into the staging directory.
//!
//! The read loop hands every `BLOB_START`, `BLOB_CHUNK`, `BLOB_END` and `Aborted` to one
//! [`Incoming`]. At most one blob is open at a time, its bytes go to a `.partial` file as
//! they arrive, and only content that matched its hash is renamed into place — so a peer
//! cannot plant bytes under the wrong hash, and an interrupted session leaves nothing
//! behind.

use sapphire_sync::{ContentHash, Stager, StagingWriter};

use crate::error::{Error, Result};

/// The blob being received, if any.
///
/// Dropping it with a blob open discards the partial file.
pub(crate) struct Incoming {
    stager: Stager,
    open: Option<(ContentHash, StagingWriter)>,
}

impl Incoming {
    /// Receive into `stager`, with nothing open.
    pub(crate) fn new(stager: Stager) -> Self {
        Self { stager, open: None }
    }

    /// `BLOB_START`: open the content `hash`, `len` bytes long.
    ///
    /// A protocol error while another blob is open, or if `len` is over this replica's
    /// limit.
    pub(crate) fn start(&mut self, hash: ContentHash, len: u64) -> Result<()> {
        if let Some((open, _)) = &self.open {
            return Err(Error::Protocol(format!(
                "content {hash} started while {open} is still open"
            )));
        }
        let writer = self.stager.writer(hash, len).map_err(|e| match e {
            sapphire_sync::Error::TooLarge { len, max } => Error::Protocol(format!(
                "content of {len} bytes exceeds the {max}-byte limit"
            )),
            other => Error::Sync(other.to_string()),
        })?;
        self.open = Some((hash, writer));
        Ok(())
    }

    /// `BLOB_CHUNK`: append to the open blob. A protocol error if none is open, or if the
    /// chunk runs past the announced length.
    pub(crate) fn chunk(&mut self, bytes: &[u8]) -> Result<()> {
        let Some((hash, writer)) = &mut self.open else {
            return Err(Error::Protocol(
                "a content chunk with no content open".to_owned(),
            ));
        };
        writer.write(bytes).map_err(|e| match e {
            sapphire_sync::Error::TooLarge { .. } => {
                Error::Protocol(format!("content {hash} ran past its announced length"))
            }
            other => Error::Sync(other.to_string()),
        })
    }

    /// `BLOB_END`: verify the open blob and stage it, returning its hash. A protocol error
    /// if none is open, or if what arrived is not the announced content.
    pub(crate) fn end(&mut self) -> Result<ContentHash> {
        let Some((hash, writer)) = self.open.take() else {
            return Err(Error::Protocol(
                "a content end with no content open".to_owned(),
            ));
        };
        writer.finish().map_err(|e| match e {
            sapphire_sync::Error::ContentMismatch { .. } => {
                Error::Protocol(format!("content does not match {hash}"))
            }
            other => Error::Sync(other.to_string()),
        })?;
        Ok(hash)
    }

    /// `Aborted(hash)`: the sender gave up on the open blob; discard it. A protocol error
    /// unless `hash` is the blob that is open.
    pub(crate) fn aborted(&mut self, hash: ContentHash) -> Result<()> {
        match &self.open {
            Some((open, _)) if *open == hash => {
                self.open = None;
                Ok(())
            }
            _ => Err(Error::Protocol(format!(
                "content {hash} was aborted but is not the content open"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming(dir: &tempfile::TempDir) -> (Incoming, Stager) {
        let stager = Stager::new(dir.path().join("staging"), 1024);
        (Incoming::new(stager.clone()), stager)
    }

    fn partials(dir: &tempfile::TempDir) -> Vec<String> {
        std::fs::read_dir(dir.path().join("staging"))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.ends_with(".partial"))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_protocol<T: std::fmt::Debug>(result: Result<T>) -> bool {
        matches!(result, Err(Error::Protocol(_)))
    }

    #[test]
    fn a_blob_is_staged_on_end() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, stager) = incoming(&dir);
        let hash = ContentHash::of_bytes(b"abc");

        incoming.start(hash, 3).unwrap();
        incoming.chunk(b"ab").unwrap();
        incoming.chunk(b"c").unwrap();
        assert_eq!(incoming.end().unwrap(), hash);
        assert!(stager.staged(&hash).is_some());
    }

    #[test]
    fn a_second_start_while_open_is_a_protocol_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, _) = incoming(&dir);
        incoming.start(ContentHash::of_bytes(b"a"), 1).unwrap();
        assert!(is_protocol(incoming.start(ContentHash::of_bytes(b"b"), 1)));
    }

    #[test]
    fn a_chunk_with_nothing_open_is_a_protocol_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, _) = incoming(&dir);
        assert!(is_protocol(incoming.chunk(b"x")));
    }

    #[test]
    fn an_end_with_nothing_open_is_a_protocol_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, _) = incoming(&dir);
        assert!(is_protocol(incoming.end()));
    }

    #[test]
    fn an_abort_discards_the_partial() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, stager) = incoming(&dir);
        let hash = ContentHash::of_bytes(b"abc");
        incoming.start(hash, 3).unwrap();
        incoming.chunk(b"a").unwrap();

        incoming.aborted(hash).unwrap();
        assert!(partials(&dir).is_empty());
        assert!(stager.staged(&hash).is_none());
        // And nothing is open any more.
        assert!(is_protocol(incoming.end()));
    }

    #[test]
    fn an_abort_for_another_hash_is_a_protocol_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, _) = incoming(&dir);
        incoming.start(ContentHash::of_bytes(b"a"), 1).unwrap();
        assert!(is_protocol(incoming.aborted(ContentHash::of_bytes(b"b"))));
    }

    #[test]
    fn a_mismatched_end_is_a_protocol_error_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, stager) = incoming(&dir);
        let hash = ContentHash::of_bytes(b"abc");
        incoming.start(hash, 3).unwrap();
        incoming.chunk(b"abd").unwrap();

        assert!(is_protocol(incoming.end()));
        assert!(stager.staged(&hash).is_none());
        assert!(partials(&dir).is_empty());
    }

    #[test]
    fn dropping_mid_blob_leaves_no_partial() {
        let dir = tempfile::tempdir().unwrap();
        let (mut incoming, _) = incoming(&dir);
        incoming.start(ContentHash::of_bytes(b"abc"), 3).unwrap();
        incoming.chunk(b"a").unwrap();
        assert_eq!(partials(&dir).len(), 1);

        drop(incoming);
        assert!(partials(&dir).is_empty());
    }
}
