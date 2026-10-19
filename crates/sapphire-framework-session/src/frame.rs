//! Framing: one tag byte, a four-byte big-endian length, then the payload.
//!
//! Not the line framing of `sapphire-framework-ipc`: content is bytes, and base64 inside
//! JSON would inflate every file by a third to no purpose.

use sapphire_sync::ContentHash;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Error, Result};
use crate::message::Message;
use crate::{CHUNK, MAX_FRAME_LEN};

const TAG_CONTROL: u8 = 0;
const TAG_BLOB_START: u8 = 1;
const TAG_BLOB_CHUNK: u8 = 2;
const TAG_BLOB_END: u8 = 3;
const HASH_LEN: usize = 32;
/// A `BLOB_START` payload: the hash, then the length as a big-endian `u64`.
const BLOB_START_LEN: usize = HASH_LEN + 8;

/// One frame off the wire.
///
/// Content travels as one `BlobStart`, its chunks in order, then `BlobEnd` — or a
/// [`Message::Aborted`] in place of the end. Control frames may come between the chunks.
#[derive(Debug)]
pub enum Frame {
    /// A control message.
    Control(Message),
    /// The start of a piece of content.
    BlobStart {
        /// The content's hash, which the receiver must verify once it is all in.
        hash: ContentHash,
        /// How many bytes the chunks that follow add up to.
        len: u64,
    },
    /// The next piece of the content started last, at most [`CHUNK`] bytes.
    BlobChunk(Vec<u8>),
    /// The content started last is complete.
    BlobEnd,
}

/// Send a control message.
pub async fn write_control<W>(w: &mut W, msg: &Message) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let payload = serde_json::to_vec(msg)?;
    write_framed(w, TAG_CONTROL, &payload).await
}

/// Start sending the content `hash`, `len` bytes long.
pub async fn write_blob_start<W>(w: &mut W, hash: &ContentHash, len: u64) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut payload = [0u8; BLOB_START_LEN];
    payload[..HASH_LEN].copy_from_slice(&hash.0);
    payload[HASH_LEN..].copy_from_slice(&len.to_be_bytes());
    write_framed(w, TAG_BLOB_START, &payload).await
}

/// Send the next piece of the content started last. More than [`CHUNK`] bytes is
/// [`Error::FrameTooLarge`], and nothing is written.
pub async fn write_blob_chunk<W>(w: &mut W, bytes: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if bytes.len() > CHUNK {
        return Err(Error::FrameTooLarge {
            len: bytes.len(),
            max: CHUNK,
        });
    }
    write_framed(w, TAG_BLOB_CHUNK, bytes).await
}

/// Say the content started last is complete.
pub async fn write_blob_end<W>(w: &mut W) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    write_framed(w, TAG_BLOB_END, &[]).await
}

/// Send one frame: `tag`, then a four-byte big-endian length, then `payload`.
///
/// Public so a protocol riding on the same framing — the pairing exchange — writes with
/// it rather than growing a second copy of the format. The payload is bounded by
/// [`MAX_FRAME_LEN`] and flushed before returning.
pub async fn write_framed<W>(w: &mut W, tag: u8, payload: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge {
            len: payload.len(),
            max: MAX_FRAME_LEN,
        });
    }
    w.write_u8(tag).await?;
    w.write_u32(payload.len() as u32).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}

/// Read one frame. `None` at a clean end of stream.
///
/// The length each tag announces is checked before anything is allocated: a control frame
/// may be up to [`MAX_FRAME_LEN`] and a chunk up to [`CHUNK`] ([`Error::FrameTooLarge`]
/// past either), while a `BLOB_START` is exactly a hash and a length and a `BLOB_END` is
/// empty ([`Error::Protocol`] otherwise).
pub async fn read_frame<R>(r: &mut R) -> Result<Option<Frame>>
where
    R: AsyncRead + Unpin,
{
    let tag = match r.read_u8().await {
        Ok(tag) => tag,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
    };
    let len = r.read_u32().await? as usize;
    // Check before allocating: a peer that announces 4 GiB must not make us try.
    let fits = |max: usize| {
        if len > max {
            Err(Error::FrameTooLarge { len, max })
        } else {
            Ok(())
        }
    };
    let exactly = |want: usize, name: &str| {
        if len != want {
            Err(Error::Protocol(format!(
                "a {name} frame of {len} bytes, expected {want}"
            )))
        } else {
            Ok(())
        }
    };
    match tag {
        TAG_CONTROL => fits(MAX_FRAME_LEN)?,
        TAG_BLOB_START => exactly(BLOB_START_LEN, "BLOB_START")?,
        TAG_BLOB_CHUNK => fits(CHUNK)?,
        TAG_BLOB_END => exactly(0, "BLOB_END")?,
        other => return Err(Error::Protocol(format!("unknown frame tag {other}"))),
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;

    Ok(Some(match tag {
        TAG_CONTROL => Frame::Control(serde_json::from_slice(&payload)?),
        TAG_BLOB_START => {
            let mut hash = [0u8; HASH_LEN];
            hash.copy_from_slice(&payload[..HASH_LEN]);
            let mut len = [0u8; 8];
            len.copy_from_slice(&payload[HASH_LEN..]);
            Frame::BlobStart {
                hash: ContentHash(hash),
                len: u64::from_be_bytes(len),
            }
        }
        TAG_BLOB_CHUNK => Frame::BlobChunk(payload),
        _ => Frame::BlobEnd,
    }))
}

/// Read one tagged frame's payload. `None` at a clean end of stream.
///
/// The caller decides what a tag means and what type the payload decodes to; this is
/// the half of [`write_framed`] a protocol other than the session needs. Reading is
/// bounded by [`MAX_FRAME_LEN`], so a peer that announces more than that is refused
/// before anything is allocated.
pub async fn read_framed<R>(r: &mut R) -> Result<Option<(u8, Vec<u8>)>>
where
    R: AsyncRead + Unpin,
{
    let tag = match r.read_u8().await {
        Ok(tag) => tag,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
    };
    let len = r.read_u32().await? as usize;
    // Check before allocating: a peer that announces 4 GiB must not make us try.
    if len > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    Ok(Some((tag, payload)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SESSION_FORMAT_VERSION;
    use sapphire_sync::{ContentHash, VersionVector};

    fn hello() -> Message {
        Message::Hello {
            format: SESSION_FORMAT_VERSION,
            workspace_id: grain_id::GrainId::random(),
            replica_id: sapphire_sync::ReplicaId::new(),
            vv: VersionVector::new(),
        }
    }

    #[tokio::test]
    async fn a_control_frame_round_trips() {
        let mut buf: Vec<u8> = Vec::new();
        let msg = hello();
        write_control(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        match read_frame(&mut cursor).await.unwrap().unwrap() {
            Frame::Control(back) => assert_eq!(format!("{back:?}"), format!("{msg:?}")),
            other => panic!("got {other:?}"),
        }
    }

    async fn read_back(buf: Vec<u8>) -> Frame {
        let mut cursor = std::io::Cursor::new(buf);
        let frame = read_frame(&mut cursor).await.unwrap().unwrap();
        assert!(read_frame(&mut cursor).await.unwrap().is_none());
        frame
    }

    /// A frame header announcing `len` bytes under `tag`, and no payload.
    fn header(tag: u8, len: usize) -> Vec<u8> {
        let mut buf = vec![tag];
        buf.extend_from_slice(&(len as u32).to_be_bytes());
        buf
    }

    #[tokio::test]
    async fn a_blob_start_round_trips() {
        let hash = ContentHash::of_bytes(b"content");
        let mut buf: Vec<u8> = Vec::new();
        write_blob_start(&mut buf, &hash, 1 << 30).await.unwrap();
        match read_back(buf).await {
            Frame::BlobStart { hash: back, len } => {
                assert_eq!(back, hash);
                assert_eq!(len, 1 << 30);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_blob_chunk_round_trips() {
        let content: Vec<u8> = (0..CHUNK).map(|i| i as u8).collect();
        let mut buf: Vec<u8> = Vec::new();
        write_blob_chunk(&mut buf, &content).await.unwrap();
        // Tag, length, content — and not a byte more.
        assert_eq!(buf.len(), 1 + 4 + CHUNK);
        match read_back(buf).await {
            Frame::BlobChunk(bytes) => assert_eq!(bytes, content),
            other => panic!("got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_blob_end_round_trips() {
        let mut buf: Vec<u8> = Vec::new();
        write_blob_end(&mut buf).await.unwrap();
        assert!(matches!(read_back(buf).await, Frame::BlobEnd));
    }

    #[tokio::test]
    async fn a_chunk_over_the_limit_is_refused_before_allocating() {
        // The header alone: a reader that trusted the length would wait for the payload.
        let mut cursor = std::io::Cursor::new(header(TAG_BLOB_CHUNK, CHUNK + 1));
        let err = read_frame(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, Error::FrameTooLarge { len, max } if len == CHUNK + 1 && max == CHUNK),
            "got {err:?}"
        );

        // And it is not written either.
        let mut buf: Vec<u8> = Vec::new();
        let err = write_blob_chunk(&mut buf, &vec![0u8; CHUNK + 1])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::FrameTooLarge { .. }), "got {err:?}");
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn a_blob_start_of_the_wrong_size_is_refused() {
        let mut buf = header(TAG_BLOB_START, 39);
        buf.extend_from_slice(&[0u8; 39]);
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(
            read_frame(&mut cursor).await.unwrap_err(),
            Error::Protocol(_)
        ));
    }

    #[tokio::test]
    async fn several_frames_read_back_in_order() {
        let mut buf: Vec<u8> = Vec::new();
        write_control(&mut buf, &hello()).await.unwrap();
        write_control(&mut buf, &Message::Done).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(
            read_frame(&mut cursor).await.unwrap(),
            Some(Frame::Control(Message::Hello { .. }))
        ));
        assert!(matches!(
            read_frame(&mut cursor).await.unwrap(),
            Some(Frame::Control(Message::Done))
        ));
        assert!(read_frame(&mut cursor).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_control_frame_over_16_mib_is_refused_before_allocating() {
        // Tag 0, length just over the limit, and nothing else: a reader that trusted the
        // length would try to allocate 16 MiB + 1 before discovering the stream is empty.
        assert_eq!(MAX_FRAME_LEN, 16 * 1024 * 1024);
        let mut cursor = std::io::Cursor::new(header(TAG_CONTROL, MAX_FRAME_LEN + 1));
        let err = read_frame(&mut cursor).await.unwrap_err();
        assert!(matches!(err, Error::FrameTooLarge { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn an_unknown_tag_is_a_protocol_error() {
        let mut buf = vec![99u8];
        buf.extend_from_slice(&0u32.to_be_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(
            read_frame(&mut cursor).await.unwrap_err(),
            Error::Protocol(_)
        ));
    }

    #[tokio::test]
    async fn a_truncated_frame_is_an_error_not_a_silent_short_read() {
        let mut buf = vec![0u8];
        buf.extend_from_slice(&64u32.to_be_bytes());
        buf.extend_from_slice(b"only ten b");

        let mut cursor = std::io::Cursor::new(buf);
        assert!(read_frame(&mut cursor).await.is_err());
    }

    #[tokio::test]
    async fn a_raw_frame_round_trips_through_the_public_pair() {
        let mut buf: Vec<u8> = Vec::new();
        write_framed(&mut buf, 9, b"payload").await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(
            read_framed(&mut cursor).await.unwrap(),
            Some((9u8, b"payload".to_vec()))
        );
        // And a clean end of stream is `None`, as for `read_frame`.
        assert!(read_framed(&mut cursor).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_raw_frame_that_is_too_large_to_write_is_refused() {
        let mut buf: Vec<u8> = Vec::new();
        let err = write_framed(&mut buf, 9, &vec![0u8; MAX_FRAME_LEN + 1])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::FrameTooLarge { .. }), "got {err:?}");
        assert!(buf.is_empty(), "nothing must be written: {buf:?}");
    }

    #[tokio::test]
    async fn a_raw_frame_that_announces_too_much_is_refused_before_allocating() {
        let mut buf = vec![9u8];
        buf.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(
            read_framed(&mut cursor).await.unwrap_err(),
            Error::FrameTooLarge { .. }
        ));
    }
}
