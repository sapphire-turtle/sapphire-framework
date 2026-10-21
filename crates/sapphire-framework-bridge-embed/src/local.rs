//! Qwen3-VL-Embedding-2B on CPU, through the [`qwen3vl`](crate::qwen3vl) port: texts, and
//! images into the same space.
//!
//! A text is truncated by tokens first, then wrapped in the template ([`wrap`]); the model's
//! tokenizer truncates at `max_tokens + 64`, so the template is never cut. An image is
//! resized to at most `max_tokens` tokens and takes the text's place in the same template.
//! The vision tower loads with the first image.
//!
//! The files are fetched through hf-hub into an explicit cache directory, never `HF_HOME` or
//! the working directory. The weights are memory-mapped and converted one tensor at a time
//! to the device's [`QType`], so the resident size is the quantized one.

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use hf_hub::api::sync::{ApiBuilder, ApiRepo};
use tokenizers::{PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::qwen3vl::preprocess::{self, PreprocessorConfig};
use crate::qwen3vl::text::{self, TextModel};
use crate::qwen3vl::vision::{VisionConfig, VisionModel};
use crate::service::Embed;
use crate::template::{mrl, truncate_at, wrap};
use crate::{Error, Result};
use sapphire_bridge_api::{LocalModel, QType};

/// Room for the template's tokens on top of `max_tokens`.
const TEMPLATE_TOKENS: usize = 64;

/// Texts per forward pass. Inputs are padded to the longest in a pass, and attention memory
/// grows with batch × length², so passes stay small even when the service hands over 32.
const FORWARD_BATCH: usize = 8;

/// A Qwen3-VL-Embedding model with this crate's template, truncation and MRL.
pub struct LocalQwen {
    name: String,
    model: TextModel,
    /// The vision tower and its preprocessing, once an image has been embedded.
    vision: Option<(VisionModel, PreprocessorConfig)>,
    /// What the vision tower loads from: the weights, `config.json`'s and the repository.
    weights: VarBuilder<'static>,
    vision_config: VisionConfig,
    image_token_id: u32,
    repo: ApiRepo,
    qtype: QType,
    /// The model's tokenizer, without the padding and truncation of `model_tokenizer`.
    tokenizer: Tokenizer,
    /// Left padding and truncation at `max_tokens + TEMPLATE_TOKENS`.
    model_tokenizer: Tokenizer,
    max_tokens: usize,
    dimension: usize,
}

impl LocalQwen {
    /// Load `settings.model`'s text tower on the CPU, its weights held as `qtype`. The files
    /// are downloaded into `cache_dir` (hf-hub layout) on first use: about 4 GB, whatever the
    /// `qtype`.
    pub fn load(settings: &LocalModel, qtype: QType, cache_dir: &Path) -> Result<Self> {
        let name = settings.model.clone();
        let load_err =
            |what: &str, e: &dyn std::fmt::Display| Error::Load(format!("{name}: {what}: {e}"));
        let mut builder = ApiBuilder::new()
            .with_cache_dir(cache_dir.to_path_buf())
            .with_progress(false);
        if let Some(token) = std::env::var("HF_TOKEN")
            .ok()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
        {
            builder = builder.with_token(Some(token));
        }
        let repo = builder
            .build()
            .map_err(|e| load_err("hf-hub", &e))?
            .model(name.clone());

        let config_path = repo
            .get("config.json")
            .map_err(|e| load_err("config.json", &e))?;
        let config_bytes = std::fs::read(&config_path).map_err(|e| load_err("config.json", &e))?;
        let config = parse_config(&name, &config_bytes)?;
        let weights = safetensors_weight_files(&name, &repo)?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| load_err("tokenizer.json", &e))?;

        // SAFETY: the safetensors files are memory-mapped read-only. They live in the hf-hub
        // cache, which nothing in this process writes while the model is loaded.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weights, DType::F32, &Device::Cpu)
                .map_err(|e| load_err("weights", &e))?
        };
        let model = TextModel::new(&config.text_config, qtype, vb.pp("model.language_model"))
            .map_err(|e| load_err("model", &e))?;

        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| load_err("tokenizer.json", &e))?;
        let mut model_tokenizer = tokenizer.clone();
        // Left padding keeps the real last token at the last position (last-token pooling).
        model_tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: PaddingDirection::Left,
            ..Default::default()
        }));
        model_tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: settings.max_tokens + TEMPLATE_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| load_err("tokenizer truncation", &e))?;

        Ok(Self {
            name,
            model,
            vision: None,
            weights: vb,
            vision_config: config.vision_config,
            image_token_id: config.image_token_id,
            repo,
            qtype,
            tokenizer,
            model_tokenizer,
            max_tokens: settings.max_tokens,
            dimension: settings.dimension as usize,
        })
    }

    /// `text` truncated to `max_tokens` content tokens and wrapped in the template.
    fn prompt(&self, text: &str) -> Result<String> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| Error::Embed(format!("tokenize: {e}")))?;
        Ok(wrap(truncate_at(
            text,
            encoding.get_offsets(),
            self.max_tokens,
        )))
    }

    /// One forward pass over `prompts`: the full-length, L2-normalized vectors.
    fn forward(&self, prompts: &[String]) -> candle_core::Result<Vec<Vec<f32>>> {
        let encodings = self
            .model_tokenizer
            .encode_batch(prompts.to_vec(), true)
            .map_err(candle_core::Error::msg)?;
        let (b, t) = (encodings.len(), encodings[0].len());
        let ids: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_ids().to_vec())
            .collect();
        let mask: Vec<f32> = encodings
            .iter()
            .flat_map(|e| e.get_attention_mask().iter().map(|&m| m as f32))
            .collect();
        let device = self.model.device();
        let ids = Tensor::from_vec(ids, (b, t), device)?;
        let mask = Tensor::from_vec(mask, (b, t), device)?;
        let positions = Tensor::arange(0u32, t as u32, device)?
            .unsqueeze(0)?
            .repeat((b, 1))?;
        let hidden = self.model.forward(
            &self.model.embed_tokens(&ids)?,
            &text::attention_mask(&mask)?,
            &positions,
            &[],
        )?;
        // Left padding: the last position is every row's real last token.
        text::l2_normalize(&hidden.i((.., t - 1))?)?.to_vec2::<f32>()
    }

    /// Load the vision tower and its preprocessing, unless they are loaded.
    fn load_vision(&mut self) -> Result<()> {
        if self.vision.is_none() {
            let err = |what: &str, e: &dyn std::fmt::Display| {
                Error::Load(format!("{}: {what}: {e}", self.name))
            };
            let path = self
                .repo
                .get("preprocessor_config.json")
                .map_err(|e| err("preprocessor_config.json", &e))?;
            let bytes = std::fs::read(path).map_err(|e| err("preprocessor_config.json", &e))?;
            let preprocessor: PreprocessorConfig =
                serde_json::from_slice(&bytes).map_err(|e| err("preprocessor_config.json", &e))?;
            let started = std::time::Instant::now();
            let model = VisionModel::new(
                &self.vision_config,
                self.qtype,
                self.weights.pp("model.visual"),
            )
            .map_err(|e| err("vision model", &e))?;
            tracing::info!("vision model loaded in {:?}", started.elapsed());
            self.vision = Some((model, preprocessor));
        }
        Ok(())
    }

    /// One image's full-length, L2-normalized vector.
    fn forward_image(&mut self, bytes: &[u8]) -> Result<Vec<f32>> {
        let image =
            image::load_from_memory(bytes).map_err(|e| Error::Embed(format!("image: {e}")))?;
        self.load_vision()?;
        let (vision, preprocessor) = self.vision.as_ref().expect("loaded above");
        let patches = preprocess::patches(&image, preprocessor, self.max_tokens)
            .map_err(|e| Error::Embed(format!("image: {e}")))?;
        let encoding = self
            .tokenizer
            .encode(preprocess::image_prompt(patches.tokens), true)
            .map_err(|e| Error::Embed(format!("tokenize: {e}")))?;
        let ids = encoding.get_ids();
        let Some(start) = ids.iter().position(|&id| id == self.image_token_id) else {
            return Err(Error::Embed("the image prompt has no image tokens".into()));
        };
        let end = start + patches.tokens;
        if ids[start..]
            .iter()
            .take_while(|&&id| id == self.image_token_id)
            .count()
            != patches.tokens
        {
            return Err(Error::Embed(
                "the image prompt's tokens do not match the image".into(),
            ));
        }
        let [gt, gh, gw] = patches.grid;
        let merge = preprocessor.merge_size;
        let positions = preprocess::image_positions(ids.len(), start, [gt, gh / merge, gw / merge]);
        let run = || -> candle_core::Result<Vec<f32>> {
            let device = self.model.device();
            let patch_len = patches.values.len() / (gt * gh * gw);
            let pixels = Tensor::from_slice(&patches.values, (gt * gh * gw, patch_len), device)?;
            let (image_tokens, deepstack) = vision.forward(&pixels, &[patches.grid])?;

            let t = ids.len();
            let ids = Tensor::from_slice(ids, (1, t), device)?;
            let text_embeds = self.model.embed_tokens(&ids)?.squeeze(0)?;
            let hidden = text_embeds.dim(1)?;
            // The image's tokens replace the placeholders; DeepStack adds only there.
            let splice =
                |middle: &Tensor, around: Option<&Tensor>| -> candle_core::Result<Tensor> {
                    let (before, after) = match around {
                        Some(base) => (base.narrow(0, 0, start)?, base.narrow(0, end, t - end)?),
                        None => (
                            Tensor::zeros((start, hidden), DType::F32, device)?,
                            Tensor::zeros((t - end, hidden), DType::F32, device)?,
                        ),
                    };
                    Tensor::cat(&[&before, middle, &after], 0)?.unsqueeze(0)
                };
            let inputs = splice(&image_tokens, Some(&text_embeds))?;
            let deepstack = deepstack
                .iter()
                .map(|feature| splice(feature, None))
                .collect::<candle_core::Result<Vec<_>>>()?;
            let positions: Vec<u32> = (0..3)
                .flat_map(|axis| positions.iter().map(move |p| p[axis]))
                .collect();
            let positions = Tensor::from_vec(positions, (3, 1, t), device)?;
            let mask = text::attention_mask(&Tensor::ones((1, t), DType::F32, device)?)?;
            let out = self.model.forward(&inputs, &mask, &positions, &deepstack)?;
            text::l2_normalize(&out.i((.., t - 1))?)?
                .squeeze(0)?
                .to_vec1::<f32>()
        };
        run().map_err(|e| Error::Embed(format!("{}: {e}", self.name)))
    }
}

/// A Qwen3-VL `config.json`: the text model lives under `model.language_model`, the vision
/// tower under `model.visual`.
#[derive(serde::Deserialize)]
struct VlConfig {
    text_config: text::Config,
    vision_config: VisionConfig,
    image_token_id: u32,
}

fn parse_config(name: &str, bytes: &[u8]) -> Result<VlConfig> {
    serde_json::from_slice(bytes)
        .map_err(|e| Error::Load(format!("{name}: config.json is not a Qwen3-VL config: {e}")))
}

/// `model.safetensors`, or every shard listed in `model.safetensors.index.json`.
/// Mirrors fastembed 7.1.1's private helper.
fn safetensors_weight_files(name: &str, repo: &ApiRepo) -> Result<Vec<PathBuf>> {
    const SINGLE_FILE: &str = "model.safetensors";
    const INDEX_FILE: &str = "model.safetensors.index.json";

    if let Ok(path) = repo.get(SINGLE_FILE) {
        return Ok(vec![path]);
    }
    let err = |e: &dyn std::fmt::Display| Error::Load(format!("{name}: {INDEX_FILE}: {e}"));
    let index_path = repo.get(INDEX_FILE).map_err(|e| err(&e))?;
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(index_path).map_err(|e| err(&e))?)
            .map_err(|e| err(&e))?;
    let mut shards: Vec<String> = index["weight_map"]
        .as_object()
        .ok_or_else(|| err(&"no `weight_map` object"))?
        .values()
        .filter_map(|file| file.as_str().map(str::to_owned))
        .collect();
    shards.sort_unstable();
    shards.dedup();
    shards
        .iter()
        .map(|file| {
            repo.get(file)
                .map_err(|e| Error::Load(format!("{name}: {file}: {e}")))
        })
        .collect()
}

impl Embed for LocalQwen {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let prompts = texts
            .iter()
            .map(|t| self.prompt(t))
            .collect::<Result<Vec<String>>>()?;
        let mut out = Vec::with_capacity(prompts.len());
        for batch in prompts.chunks(FORWARD_BATCH) {
            let vectors = self
                .forward(batch)
                .map_err(|e| Error::Embed(format!("{}: {e}", self.name)))?;
            out.extend(vectors.iter().map(|v| mrl(v, self.dimension)));
        }
        Ok(out)
    }

    fn embed_images(&mut self, images: &[Vec<u8>]) -> Result<Vec<Vec<f32>>> {
        images
            .iter()
            .map(|bytes| Ok(mrl(&self.forward_image(bytes)?, self.dimension)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    /// `SAPPHIRE_EMBED_TEST_CACHE`, else the default model cache,
    /// `dirs::cache_dir()/sapphire-bridge/models`.
    fn cache() -> PathBuf {
        std::env::var("SAPPHIRE_EMBED_TEST_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::cache_dir()
                    .expect("no platform cache dir")
                    .join("sapphire-bridge")
                    .join("models")
            })
    }

    /// This process's anonymous resident memory, from `/proc` (Linux only): the model,
    /// without the memory-mapped weight files.
    fn resident() -> String {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("RssAnon:"))
                    .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            })
            .unwrap_or_else(|| "RssAnon: unknown".into())
    }

    /// How long eight texts of about 500 tokens and one 1-megapixel image take, and the
    /// memory after each. At `SAPPHIRE_EMBED_TEST_QTYPE` (default f16).
    #[test]
    #[ignore]
    fn real_model_speed() {
        let (settings, qtype) = settings_from_env();
        let started = std::time::Instant::now();
        let mut model = LocalQwen::load(&settings, qtype, &cache()).unwrap();
        eprintln!(
            "{qtype}: text loaded in {:?}, {}",
            started.elapsed(),
            resident()
        );
        let long = "雨の日は家で本を読むのが好きです。".repeat(40);
        let input: Vec<String> = (0..8).map(|i| format!("{i}: {long}")).collect();
        let started = std::time::Instant::now();
        model.embed(&input).unwrap();
        eprintln!(
            "{qtype}: 8 texts in {:?}, {}",
            started.elapsed(),
            resident()
        );
        let started = std::time::Instant::now();
        model.load_vision().unwrap();
        eprintln!(
            "{qtype}: vision loaded in {:?}, {}",
            started.elapsed(),
            resident()
        );
        let started = std::time::Instant::now();
        model.embed_images(&[shape_png(true, 1280, 800)]).unwrap();
        eprintln!(
            "{qtype}: 1 image in {:?}, {}",
            started.elapsed(),
            resident()
        );
    }

    /// The default model, at `SAPPHIRE_EMBED_TEST_QTYPE` (default f16).
    fn settings_from_env() -> (LocalModel, QType) {
        let qtype = std::env::var("SAPPHIRE_EMBED_TEST_QTYPE")
            .map(|q| q.parse().unwrap())
            .unwrap_or_default();
        (LocalModel::default(), qtype)
    }

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// A PNG of `width × height`: a red disc on white, or a blue square on white.
    fn shape_png(red_disc: bool, width: u32, height: u32) -> Vec<u8> {
        let (cx, cy, r) = (
            width as f32 / 2.0,
            height as f32 / 2.0,
            width.min(height) as f32 / 3.0,
        );
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let inside = if red_disc {
                dx * dx + dy * dy < r * r
            } else {
                dx.abs() < r && dy.abs() < r
            };
            match (inside, red_disc) {
                (false, _) => image::Rgb([255, 255, 255]),
                (true, true) => image::Rgb([220, 20, 20]),
                (true, false) => image::Rgb([20, 40, 220]),
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A text query finds the matching image: the vision tower and the text tower share a
    /// space. Uses `SAPPHIRE_EMBED_TEST_QTYPE` (default f16).
    #[test]
    #[ignore]
    fn real_model_matches_images_to_text() {
        let (settings, qtype) = settings_from_env();
        let mut model = LocalQwen::load(&settings, qtype, &cache()).unwrap();
        let started = std::time::Instant::now();
        let images = model
            .embed_images(&[shape_png(true, 320, 240), shape_png(false, 320, 240)])
            .unwrap();
        eprintln!(
            "{qtype}: two images in {:?}, text and vision loaded, {}",
            started.elapsed(),
            resident()
        );
        let queries = model.embed(&texts(&["赤い丸", "a blue square"])).unwrap();
        for v in images.iter().chain(&queries) {
            assert_eq!(v.len(), 1024);
            assert!((dot(v, v).sqrt() - 1.0).abs() < 1e-4);
        }
        let s = |q: usize, i: usize| dot(&queries[q], &images[i]);
        eprintln!(
            "{qtype}: red/disc {:.4} red/square {:.4} blue/disc {:.4} blue/square {:.4}",
            s(0, 0),
            s(0, 1),
            s(1, 0),
            s(1, 1)
        );
        assert!(s(0, 0) > s(0, 1));
        assert!(s(1, 1) > s(1, 0));
    }

    /// Downloads about 4 GB on first run. Resident: about 3.5 GB at f16, 7 GB at f32.
    #[test]
    #[ignore]
    fn real_model_embeds_japanese_sensibly() {
        let (settings, qtype) = settings_from_env();
        let mut model = LocalQwen::load(&settings, qtype, &cache()).unwrap();
        eprintln!("{qtype}: text model loaded, {}", resident());
        let v = model
            .embed(&texts(&[
                "今日は雨が降っている",
                "雨の日です",
                "請求書の支払い期限",
            ]))
            .unwrap();
        assert_eq!(v.len(), 3);
        for x in &v {
            assert_eq!(x.len(), 1024);
            assert!((dot(x, x).sqrt() - 1.0).abs() < 1e-4);
        }
        let rain = dot(&v[0], &v[1]);
        assert!(
            rain > dot(&v[0], &v[2]),
            "rain {rain} vs bill {}",
            dot(&v[0], &v[2])
        );
        assert!(
            rain > dot(&v[1], &v[2]),
            "rain {rain} vs bill {}",
            dot(&v[1], &v[2])
        );
        eprintln!(
            "{qtype}: rain/rain {rain:.4}, rain/bill {:.4}, rain2/bill {:.4}",
            dot(&v[0], &v[2]),
            dot(&v[1], &v[2])
        );
    }

    /// Loads the model twice, at f32 and at `SAPPHIRE_EMBED_TEST_QTYPE`: about 10 GB of RAM.
    #[test]
    #[ignore]
    fn real_model_quantized_stays_close_to_f32() {
        let (settings, qtype) = settings_from_env();
        let input = texts(&[
            "今日は雨が降っている",
            "請求書の支払い期限は月末です。",
            "The quick brown fox jumps over the lazy dog.",
        ]);
        let reference = LocalQwen::load(&settings, QType::F32, &cache())
            .unwrap()
            .embed(&input)
            .unwrap();
        let quantized = LocalQwen::load(&settings, qtype, &cache())
            .unwrap()
            .embed(&input)
            .unwrap();
        for (a, b) in reference.iter().zip(&quantized) {
            let cos = dot(a, b);
            eprintln!("{qtype} vs f32: cos {cos:.5}");
            if qtype != QType::Q4K {
                assert!(cos > 0.99, "{qtype}: cos {cos}");
            }
        }
    }
}
