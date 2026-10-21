# Local embedding: quantization, images and ANN

- Date: 2026-10-21
- Issues: #307 (quantization), #306 (images and ANN), #291 (multimodal embedding; images
  only here)
- Builds on: `2026-10-09-bridge-embedding-design.md` (#185),
  `2026-10-10-embedding-settings-design.md` (#186), `2026-10-10-synced-vectors-design.md`
  (#187)
- Related: #313 (Qwen3-VL-Embedding-8B, postponed: about 9 GB at q8_0 is too much for the
  phones this should run on), #314 (a generic `config set` CLI, not part of this work)

## Why

- The local model, `Qwen/Qwen3-VL-Embedding-2B`, is loaded at f32: about 7 GB resident.
  The bridge is a resident process, and it should one day run on phones.
- Qwen3-VL-Embedding embeds images into the same space as text, but only the text tower is
  used. Images synced into a workspace cannot be found by search.
- Vector search scans every vector. Images add many more of them.

## Decisions

1. **The model code moves into `sapphire-framework-bridge-embed`.** fastembed 7.1.1 builds
   every projection as a dense `candle_nn::Linear`, and its image embedder can only be
   loaded through its own cache directory (`FASTEMBED_CACHE_DIR` or the working
   directory). The Qwen3 text model, the Qwen3-VL vision tower and the image preprocessing
   are therefore ported from fastembed 7.1.1 (Apache-2.0) into a `qwen3vl` module, with
   every linear layer behind candle's `QMatMul`. fastembed and its ONNX runtime are no
   longer dependencies. At f32 the port gives the same text vectors bit for bit, and image
   vectors at 0.9999994 cosine similarity to fastembed's (the patch convolution, whose
   stride equals its kernel, is written as the equivalent matrix product).
2. **The quantization is a device setting, `[local] qtype`, in the device's
   `embedding.toml`.** Values: `f32`, `f16` (the default), `q8_0` and `q4_k`. It is a property of the device's memory and CPU, not of the workgroup, so it
   does not sync.
   - The weights are quantized while loading, from the same safetensors download. There is
     one download whatever the `qtype`.
   - A quantized model's vectors count as the same model's: `qtype` is no part of
     `embedding_model`, the store's identity or the vector profile. Devices with different
     `qtype`s share vectors.
   - Linear layers and the token embedding are quantized. Norms, biases, the vision
     position embedding and the patch embedding stay at f32. A layer whose input width is
     not a multiple of the block size (256 for Q4_K, 32 for Q8_0) falls back to the next
     wider type: Q4_K → Q8_0 → F16. Only the vision tower needs this (1152 and 4304 wide).
   - Measured on 2B, on 8 x86_64 cores with AVX-512: anonymous resident memory after
     loading the text tower (the vision tower adds about a fifth), eight texts of about 500
     tokens, one 1-megapixel image, and the cosine similarity of three texts to `f32`:

     | qtype | build | resident | 8 texts | 1 image | cosine to f32 |
     |---|---|---|---|---|---|
     | f32 | default | 6.9 GB | 54 s | 37 s | 1 |
     | f16 | default | 3.5 GB | 52 s | 35 s | 1.00000 |
     | q8_0 | default | 1.9 GB | 906 s | 428 s | 0.9985–0.9989 |
     | q8_0 | AVX2 | 1.9 GB | 157 s | 85 s | (same) |
     | q4_k | AVX2 | 1.1 GB | 120 s | 75 s | 0.83–0.88 |

   - candle picks its SIMD kernels for quantized types at compile time
     (`target_feature = "avx2"` / `"neon"`); its `f16` matrix products detect AVX at run
     time. So a default x86_64 build runs `q8_0` and `q4_k` on scalar code, and even a
     build for AVX2 CPUs runs them three times slower than `f16`. **The default is `f16`**:
     no loss, half of `f32`'s memory, the fastest. `q8_0` and `q4_k` are for a device short
     of memory, and the bridge warns when one is set on a build without the vector kernels
     (`QType::FAST_QUANTIZED`). On aarch64 the kernels always use NEON; phones are not
     measured yet.

3. **Images are embedded by the local model only.** The OpenAI-compatible API has no
   common image input, so the remote slot stays text-only.
   - `EmbedModelInfo` gains `images: bool` (absent from older bridges: false). The
     workspace embeds images only when it is true.
   - A new method, `embed.embed_images`, takes base64 images and returns vectors like
     `embed.embed`. `bridge-api` becomes 2.4.0.
   - The prompt is the official one with the image in place of the text. The image is
     resized to at most `max_tokens` vision tokens (each 32 × 32 pixels), so the default
     1024 tokens is about one megapixel.
   - The vision tower loads with the first image, so a device that embeds only text does
     not hold it.
4. **An image is a document like a text file.** One file is one document and one vector.
   - Extensions: `png`, `jpg`, `jpeg`, `webp`, `gif` (the first frame). Files over 20 MiB
     are skipped.
   - `Document` gains a `kind`: `Text`, or `Image { content_hash }`. An image's body is its
     file name, which full-text search sees; its search result has no snippet.
   - The indexer hashes the image once, when it indexes it. The store keys the vector by
     that hash, and reads the file only to embed it, so a sync does not read every image.
     For text the hash is the text's, as before, so existing synced vectors stay valid, and
     for images it is what the server's backfill policy already compares (the replica's
     content hash).
   - An image the embedder rejects while it embeds a text is skipped until its content
     changes or the process restarts: unreadable or undecodable files must not stop the
     rest. A batch of images that all fail is an outage only when a text fails too.
   - Video and audio are out of scope.
5. **Approximate nearest-neighbour search with `instant-distance` (HNSW).** Pure Rust,
   small, MIT/Apache. The index is in memory and derived from the redb vectors, like the
   tantivy index; it is never saved.
   - Below 10 000 vectors, with a folder, or for more than 200 candidates, the search
     stays brute force.
   - At or above 10 000, a search starts building the index on a background thread, and
     is served brute force until it is ready.
   - Vectors added or removed after the build are searched exactly next to the index and
     filtered out of its results. When they reach a tenth of the index, it is rebuilt in
     the background. A model change drops it.

## Components

- `sapphire-framework-bridge-api` 2.4.0: `QType`, `DeviceSettings::local_qtype`,
  `EmbedModelInfo::images`, `EMBED_IMAGES`, `EmbedImagesParams`; `embedding show` prints the
  device's `qtype`.
- `sapphire-framework-bridge-embed`: `qwen3vl::{text, vision, preprocess, qlinear}`;
  `LocalQwen::load(model, qtype, cache_dir)`; `Embed::embed_images` (default: unsupported),
  `EmbedService::embed_images`, one image per call in the worker.
- `sapphire-framework-bridge`: `[local] qtype` in the device file, `EmbedConfig::Local::qtype`
  (a change reloads the model), `EmbedProvider::embed_images`, the `embed.embed_images`
  route. The bridge binary passes both through.
- `sapphire-framework-bridge-client`: `BridgeClient::embed_images`.
- `sapphire-framework-retrieve`: `Embedder::{images, embed_images}`, `Document::kind`
  (stored with `#[serde(default)]`, so no schema bump), `VectorSource` over `Content`, images
  in `embed_pending`, the `ann` module.
- `sapphire-framework-workspace`: the indexer's image extensions and size limit;
  `BridgeEmbedder::embed_images`.

## Error handling

- An image that fails to decode is logged and skipped (see 4).
- A bridge older than 2.4 reports no `images`, so nothing calls `embed.embed_images` on it.
- An older primary device does not index images, and its stale-vector sweep may remove
  the image vectors newer devices wrote. They are rewritten when a newer device embeds.
- An unknown `qtype` makes the device file invalid, as any bad field does: the bridge falls
  back to the default settings and logs it, and the CLI refuses to rewrite the file.

## Out of scope

- The 8B model (#313), video and audio (#291 stays open for them), images through the
  remote slot, image queries.
- Editing `qtype` from the CLI or the GUI: it is set in the file; #314 adds a generic
  `config set`.

## Testing

- Unit tests: `qtype` parsing, the device file's `qtype`, `QLinear` fallback by block size
  and closeness to f32, the RoPE sections, image preprocessing (resize bounds, patch order,
  positions), the service's image jobs, image documents in the indexer and in
  `embed_pending` (text-only embedder, synced vector by hash, bad images), the ANN index
  (recall against brute force, changes during and after a build, a model change).
- Ignored real-model tests (`SAPPHIRE_EMBED_TEST_QTYPE`): Japanese similarity, the quantized
  model against f32, a text query ranking the matching image first, and timings with the
  resident memory.
