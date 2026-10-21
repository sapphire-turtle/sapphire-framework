//! The embedding interface.
//!
//! Converts text, and with some models images, into float vectors in one space, used for
//! semantic similarity search.
//! This crate only defines the [`Embedder`] trait; the providers (local
//! inference and OpenAI-compatible REST) live in the bridge's embedding
//! component (`sapphire-framework-bridge-embed`).

use crate::error::Result;

/// Abstraction over a text embedding provider.
pub trait Embedder: Send + Sync {
    /// Generate embeddings for a batch of texts.
    ///
    /// Returns one `Vec<f32>` per input text, in the same order.
    /// Returns an empty `Vec` when `texts` is empty.
    fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Whether [`embed_images`](Self::embed_images) works.
    fn images(&self) -> bool {
        false
    }

    /// Generate embeddings for images (each a file's bytes), in the texts' space.
    ///
    /// Returns one `Vec<f32>` per image, in the same order. Only an embedder whose
    /// [`images`](Self::images) is true supports it.
    fn embed_images(&self, images: &[&[u8]]) -> Result<Vec<Vec<f32>>> {
        let _ = images;
        Err(crate::Error::Embed(
            "this embedder cannot embed images".into(),
        ))
    }
}
