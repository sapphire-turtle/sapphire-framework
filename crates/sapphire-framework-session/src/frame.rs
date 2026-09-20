//! Framing: one tag byte, a four-byte big-endian length, then the payload.
//!
//! Not the line framing of `sapphire-framework-ipc`: content is bytes, and base64 inside
//! JSON would inflate every file by a third to no purpose.

use sapphire_sync::ContentHash;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::MAX_FRAME_LEN;
use crate::error::{Error, Result};
use crate::message::Message;

const TAG_CONTROL: u8 = 0;
const TAG_BLOB: u8 = 1;
const HASH_LEN: usize = 32;

/// One frame off the wire.
#[derive(Debug)]
pub enum Frame {
    /// A control message.
    Control(Message),
    /// Content, addressed by its hash.
    Blob {
        /// The content's hash, which the receiver must verify.
        hash: ContentHash,
        /// The content.
        bytes: Vec<u8>,
    },
}

/// Send a control message.
pub async fn write_control<W>(w: &mut W, msg: &Message) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let payload = serde_json::to_vec(msg)?;
    write_framed(w, TAG_CONTROL, &payload).await
}

/// Send content.
pub async fn write_blob<W>(w: &mut W, hash: &ContentHash, bytes: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut payload = Vec::with_capacity(HASH_LEN + bytes.len());
    payload.extend_from_slice(&hash.0);
    payload.extend_from_slice(bytes);
    write_framed(w, TAG_BLOB, &payload).await
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

/// Read one frame's tag and payload, dispatching on `tag` with `read_tagged`.
///
/// `None` at a clean end of stream.
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
    if len > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;

    match tag {
        TAG_CONTROL => Ok(Some(Frame::Control(serde_json::from_slice(&payload)?))),
        TAG_BLOB => {
            if payload.len() < HASH_LEN {
                return Err(Error::Protocol(format!(
                    "a blob frame of {} bytes cannot hold a hash",
                    payload.len()
                )));
            }
            let mut hash = [0u8; HASH_LEN];
            hash.copy_from_slice(&payload[..HASH_LEN]);
            Ok(Some(Frame::Blob {
                hash: ContentHash(hash),
                bytes: payload[HASH_LEN..].to_vec(),
            }))
        }
        other => Err(Error::Protocol(format!("unknown frame tag {other}"))),
    }
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

    #[tokio::test]
    async fn a_blob_frame_round_trips_without_growing() {
        let content = vec![0xABu8; 100_000];
        let hash = ContentHash::of_bytes(&content);

        let mut buf: Vec<u8> = Vec::new();
        write_blob(&mut buf, &hash, &content).await.unwrap();
        // Tag, length, hash, content — and not a byte more.
        assert_eq!(buf.len(), 1 + 4 + 32 + content.len());

        let mut cursor = std::io::Cursor::new(buf);
        match read_frame(&mut cursor).await.unwrap().unwrap() {
            Frame::Blob { hash: back, bytes } => {
                assert_eq!(back, hash);
                assert_eq!(bytes, content);
            }
            other => panic!("got {other:?}"),
        }
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
    async fn an_oversized_length_is_refused_before_allocating() {
        // Tag 0, length just over the limit, and nothing else: a reader that trusted the
        // length would try to allocate 64 MiB + 1 before discovering the stream is empty.
        let mut buf = vec![0u8];
        buf.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());

        let mut cursor = std::io::Cursor::new(buf);
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

    #[tokio::test]
    async fn a_blob_frame_shorter_than_its_hash_is_refused() {
        let mut buf = vec![1u8];
        buf.extend_from_slice(&8u32.to_be_bytes());
        buf.extend_from_slice(&[0u8; 8]);
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(
            read_frame(&mut cursor).await.unwrap_err(),
            Error::Protocol(_)
        ));
    }
}
