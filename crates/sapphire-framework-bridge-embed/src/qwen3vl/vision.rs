//! The Qwen3-VL vision tower: image patches → tokens for the text model, plus the DeepStack
//! features added to its first layers.
//!
//! Ported from fastembed 7.1.1 (`models/qwen3_vl.rs`). Differences: the projections are
//! [`QLinear`]s, and the patch embedding (a Conv3d whose stride equals its kernel) is the
//! same product written as one linear layer over the flattened patch.

use candle_core::{D, DType, Device, IndexOp, Module, Result, Tensor};
use candle_nn::{Activation, Embedding, LayerNorm, LayerNormConfig, VarBuilder};
use sapphire_bridge_api::QType;
use serde::Deserialize;

use super::qlinear::QLinear;

/// `config.json`'s `vision_config`.
#[derive(Debug, Clone, Deserialize)]
pub struct VisionConfig {
    pub depth: usize,
    pub hidden_size: usize,
    pub out_hidden_size: usize,
    pub hidden_act: Activation,
    pub intermediate_size: usize,
    pub num_heads: usize,
    #[serde(default = "default_in_channels")]
    pub in_channels: usize,
    pub patch_size: usize,
    pub spatial_merge_size: usize,
    pub temporal_patch_size: usize,
    pub num_position_embeddings: usize,
    #[serde(default)]
    pub deepstack_visual_indexes: Vec<usize>,
}

fn default_in_channels() -> usize {
    3
}

fn layer_norm(dim: usize, vb: VarBuilder) -> Result<LayerNorm> {
    let cfg = LayerNormConfig {
        eps: 1e-6,
        ..Default::default()
    };
    candle_nn::layer_norm(dim, cfg, vb)
}

struct Mlp {
    fc1: QLinear,
    fc2: QLinear,
    act: Activation,
}

impl Mlp {
    fn new(cfg: &VisionConfig, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let (h, i) = (cfg.hidden_size, cfg.intermediate_size);
        Ok(Self {
            fc1: QLinear::new(h, i, true, qtype, vb.pp("linear_fc1"))?,
            fc2: QLinear::new(i, h, true, qtype, vb.pp("linear_fc2"))?,
            act: cfg.hidden_act,
        })
    }

    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        xs.apply(&self.fc1)?.apply(&self.act)?.apply(&self.fc2)
    }
}

fn rotate_half(xs: &Tensor) -> Result<Tensor> {
    let last = xs.dim(D::Minus1)?;
    let xs1 = xs.narrow(D::Minus1, 0, last / 2)?;
    let xs2 = xs.narrow(D::Minus1, last / 2, last - last / 2)?;
    Tensor::cat(&[&xs2.neg()?, &xs1], D::Minus1)
}

/// `q`, `k`: `[S, H, D]`; `cos`, `sin`: `[S, D]`.
fn apply_rotary(q: &Tensor, k: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<(Tensor, Tensor)> {
    let cos = cos.unsqueeze(D::Minus2)?;
    let sin = sin.unsqueeze(D::Minus2)?;
    let q = (q.broadcast_mul(&cos)? + rotate_half(q)?.broadcast_mul(&sin)?)?;
    let k = (k.broadcast_mul(&cos)? + rotate_half(k)?.broadcast_mul(&sin)?)?;
    Ok((q, k))
}

struct Attention {
    qkv: QLinear,
    proj: QLinear,
    num_heads: usize,
    head_dim: usize,
}

impl Attention {
    fn new(cfg: &VisionConfig, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let dim = cfg.hidden_size;
        Ok(Self {
            qkv: QLinear::new(dim, dim * 3, true, qtype, vb.pp("qkv"))?,
            proj: QLinear::new(dim, dim, true, qtype, vb.pp("proj"))?,
            num_heads: cfg.num_heads,
            head_dim: dim / cfg.num_heads,
        })
    }

    /// Attention within each image (frame): `cu_seqlens` are the bounds in `xs` (`[S, H]`).
    fn forward(
        &self,
        xs: &Tensor,
        cu_seqlens: &[usize],
        cos: &Tensor,
        sin: &Tensor,
    ) -> Result<Tensor> {
        let seq_len = xs.dim(0)?;
        let qkv = xs
            .apply(&self.qkv)?
            .reshape((seq_len, 3, self.num_heads, self.head_dim))?
            .permute((1, 0, 2, 3))?;
        let (q, k) = apply_rotary(&qkv.i(0)?, &qkv.i(1)?, cos, sin)?;
        let v = qkv.i(2)?;
        let scale = (self.head_dim as f64).powf(-0.5);

        let mut outputs = Vec::with_capacity(cu_seqlens.len().saturating_sub(1));
        for window in cu_seqlens.windows(2) {
            let (start, len) = (window[0], window[1].saturating_sub(window[0]));
            if len == 0 {
                continue;
            }
            // [S, H, D] → [H, S, D]
            let chunk = |t: &Tensor| t.narrow(0, start, len)?.transpose(0, 1)?.contiguous();
            let (q, k, v) = (chunk(&q)?, chunk(&k)?, chunk(&v)?);
            let attn = (q.matmul(&k.transpose(1, 2)?)? * scale)?;
            let attn = candle_nn::ops::softmax_last_dim(&attn)?;
            outputs.push(
                attn.matmul(&v)?
                    .transpose(0, 1)?
                    .reshape((len, self.num_heads * self.head_dim))?,
            );
        }
        Tensor::cat(&outputs, 0)?.apply(&self.proj)
    }
}

struct Block {
    norm1: LayerNorm,
    norm2: LayerNorm,
    attn: Attention,
    mlp: Mlp,
}

impl Block {
    fn new(cfg: &VisionConfig, qtype: QType, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            norm1: layer_norm(cfg.hidden_size, vb.pp("norm1"))?,
            norm2: layer_norm(cfg.hidden_size, vb.pp("norm2"))?,
            attn: Attention::new(cfg, qtype, vb.pp("attn"))?,
            mlp: Mlp::new(cfg, qtype, vb.pp("mlp"))?,
        })
    }

    fn forward(
        &self,
        xs: &Tensor,
        cu_seqlens: &[usize],
        cos: &Tensor,
        sin: &Tensor,
    ) -> Result<Tensor> {
        let xs = (xs
            + self
                .attn
                .forward(&xs.apply(&self.norm1)?, cu_seqlens, cos, sin)?)?;
        let mlp = self.mlp.forward(&xs.apply(&self.norm2)?)?;
        xs + mlp
    }
}

/// Merges each `spatial_merge_size²` neighbourhood into one token of the text model's width.
struct PatchMerger {
    norm: LayerNorm,
    postshuffle_norm: bool,
    merged: usize,
    fc1: QLinear,
    fc2: QLinear,
}

impl PatchMerger {
    fn new(
        cfg: &VisionConfig,
        postshuffle_norm: bool,
        qtype: QType,
        vb: VarBuilder,
    ) -> Result<Self> {
        let merged = cfg.hidden_size * cfg.spatial_merge_size.pow(2);
        let norm_dim = if postshuffle_norm {
            merged
        } else {
            cfg.hidden_size
        };
        Ok(Self {
            norm: layer_norm(norm_dim, vb.pp("norm"))?,
            postshuffle_norm,
            merged,
            fc1: QLinear::new(merged, merged, true, qtype, vb.pp("linear_fc1"))?,
            fc2: QLinear::new(
                merged,
                cfg.out_hidden_size,
                true,
                qtype,
                vb.pp("linear_fc2"),
            )?,
        })
    }

    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let grouped = xs.elem_count() / self.merged;
        let xs = if self.postshuffle_norm {
            xs.reshape((grouped, self.merged))?.apply(&self.norm)?
        } else {
            xs.apply(&self.norm)?.reshape((grouped, self.merged))?
        };
        xs.apply(&self.fc1)?.gelu()?.apply(&self.fc2)
    }
}

/// The vision tower.
pub struct VisionModel {
    patch_embed: QLinear,
    pos_embed: Embedding,
    blocks: Vec<Block>,
    merger: PatchMerger,
    deepstack_mergers: Vec<PatchMerger>,
    deepstack_lookup: Vec<Option<usize>>,
    /// `1 / 10000^(2i / (head_dim / 2))`, `[1, head_dim / 4]`.
    inv_freq: Tensor,
    spatial_merge_size: usize,
    num_grid_per_side: usize,
    hidden_size: usize,
}

impl VisionModel {
    /// Load from `vb` (f32, at `model.visual`), each projection held as `qtype`. The patch
    /// embedding stays at f32.
    pub fn new(cfg: &VisionConfig, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let patch = cfg.in_channels * cfg.temporal_patch_size * cfg.patch_size.pow(2);
        let patch_vb = vb.pp("patch_embed").pp("proj");
        let weight = patch_vb
            .get(
                (
                    cfg.hidden_size,
                    cfg.in_channels,
                    cfg.temporal_patch_size,
                    cfg.patch_size,
                    cfg.patch_size,
                ),
                "weight",
            )?
            .reshape((cfg.hidden_size, patch))?;
        let patch_embed = QLinear::from_weight(
            weight,
            Some(patch_vb.get(cfg.hidden_size, "bias")?),
            QType::F32,
        )?;
        let pos_embed = candle_nn::embedding(
            cfg.num_position_embeddings,
            cfg.hidden_size,
            vb.pp("pos_embed"),
        )?;
        let blocks = (0..cfg.depth)
            .map(|i| Block::new(cfg, qtype, vb.pp(format!("blocks.{i}"))))
            .collect::<Result<Vec<_>>>()?;
        let merger = PatchMerger::new(cfg, false, qtype, vb.pp("merger"))?;
        let deepstack_mergers = (0..cfg.deepstack_visual_indexes.len())
            .map(|i| {
                PatchMerger::new(
                    cfg,
                    true,
                    qtype,
                    vb.pp(format!("deepstack_merger_list.{i}")),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let mut deepstack_lookup = vec![None; cfg.depth];
        for (idx, &layer) in cfg.deepstack_visual_indexes.iter().enumerate() {
            if layer < cfg.depth {
                deepstack_lookup[layer] = Some(idx);
            }
        }

        let rotary_dim = cfg.hidden_size / cfg.num_heads / 2;
        let inv_freq: Vec<f32> = (0..rotary_dim)
            .step_by(2)
            .map(|i| 1f32 / 10000f32.powf(i as f32 / rotary_dim as f32))
            .collect();
        let inv_freq = Tensor::from_vec(inv_freq.clone(), (1, inv_freq.len()), vb.device())?;

        let num_grid_per_side = (cfg.num_position_embeddings as f64).sqrt().round() as usize;
        if num_grid_per_side.pow(2) != cfg.num_position_embeddings {
            candle_core::bail!(
                "num_position_embeddings {} is not a perfect square",
                cfg.num_position_embeddings
            );
        }
        Ok(Self {
            patch_embed,
            pos_embed,
            blocks,
            merger,
            deepstack_mergers,
            deepstack_lookup,
            inv_freq,
            spatial_merge_size: cfg.spatial_merge_size,
            num_grid_per_side,
            hidden_size: cfg.hidden_size,
        })
    }

    fn linspace(&self, steps: usize) -> Vec<f32> {
        if steps == 1 {
            return vec![0.0];
        }
        let step = (self.num_grid_per_side - 1) as f32 / (steps - 1) as f32;
        (0..steps).map(|i| i as f32 * step).collect()
    }

    /// The learned position embeddings, bilinearly interpolated to each image's grid and put
    /// in the merge order of the patches.
    fn position_embeddings(&self, grids: &[[usize; 3]], device: &Device) -> Result<Tensor> {
        let side = self.num_grid_per_side;
        let mut permuted = Vec::with_capacity(grids.len());
        for &[t, h, w] in grids {
            let (hs, ws) = (self.linspace(h), self.linspace(w));
            let mut idx: [Vec<u32>; 4] = Default::default();
            let mut weight: [Vec<f32>; 4] = Default::default();
            for &hv in &hs {
                let (hf, hc) = (hv.floor() as usize, (hv.ceil() as usize).min(side - 1));
                let dh = hv - hf as f32;
                for &wv in &ws {
                    let (wf, wc) = (wv.floor() as usize, (wv.ceil() as usize).min(side - 1));
                    let dw = wv - wf as f32;
                    let corners = [
                        (hf * side + wf, (1.0 - dh) * (1.0 - dw)),
                        (hf * side + wc, (1.0 - dh) * dw),
                        (hc * side + wf, dh * (1.0 - dw)),
                        (hc * side + wc, dh * dw),
                    ];
                    for (k, (i, wt)) in corners.into_iter().enumerate() {
                        idx[k].push(i as u32);
                        weight[k].push(wt);
                    }
                }
            }
            let mut sum: Option<Tensor> = None;
            for k in 0..4 {
                let ids = Tensor::from_slice(&idx[k], h * w, device)?;
                let wt = Tensor::from_slice(&weight[k], (h * w, 1), device)?;
                let part = self.pos_embed.forward(&ids)?.broadcast_mul(&wt)?;
                sum = Some(match sum {
                    Some(s) => (s + part)?,
                    None => part,
                });
            }
            let m = self.spatial_merge_size;
            let pos = sum
                .expect("four corners")
                .repeat((t, 1))?
                .reshape((t, h / m, m, w / m, m, self.hidden_size))?
                .permute((0, 1, 3, 2, 4, 5))?
                .reshape((t * h * w, self.hidden_size))?;
            permuted.push(pos);
        }
        Tensor::cat(&permuted, 0)
    }

    /// The 2D rotary angles of every patch, in merge order: `[S, head_dim / 2]`.
    fn rotary_angles(&self, grids: &[[usize; 3]], device: &Device) -> Result<Tensor> {
        let max_hw = grids.iter().flat_map(|g| [g[1], g[2]]).max().unwrap_or(0);
        let freq_table = Tensor::arange(0f32, max_hw as f32, device)?
            .unsqueeze(D::Minus1)?
            .broadcast_matmul(&self.inv_freq)?;
        let m = self.spatial_merge_size;
        let (mut rows, mut cols) = (Vec::new(), Vec::new());
        for &[t, h, w] in grids {
            for _ in 0..t {
                for br in 0..h / m {
                    for bc in 0..w / m {
                        for ir in 0..m {
                            for ic in 0..m {
                                rows.push((br * m + ir) as u32);
                                cols.push((bc * m + ic) as u32);
                            }
                        }
                    }
                }
            }
        }
        let n = rows.len();
        let rows = freq_table.index_select(&Tensor::from_vec(rows, n, device)?, 0)?;
        let cols = freq_table.index_select(&Tensor::from_vec(cols, n, device)?, 0)?;
        Tensor::stack(&[rows, cols], D::Minus2)?.reshape((n, ()))
    }

    /// `patches` (`[S, C·T·P·P]`, in merge order) and each image's `[t, h, w]` grid of
    /// patches → `(tokens, deepstack)`: `tokens` is `[S / merge², out_hidden]`, and each
    /// DeepStack feature has the same shape.
    pub fn forward(&self, patches: &Tensor, grids: &[[usize; 3]]) -> Result<(Tensor, Vec<Tensor>)> {
        let device = patches.device();
        let mut xs = (patches.to_dtype(DType::F32)?.apply(&self.patch_embed)?
            + self.position_embeddings(grids, device)?)?;

        let angles = self.rotary_angles(grids, device)?;
        let emb = Tensor::cat(&[&angles, &angles], D::Minus1)?;
        let (cos, sin) = (emb.cos()?, emb.sin()?);

        let mut cu_seqlens = vec![0usize];
        for &[t, h, w] in grids {
            for _ in 0..t {
                cu_seqlens.push(cu_seqlens.last().copied().unwrap_or(0) + h * w);
            }
        }

        let mut deepstack = Vec::with_capacity(self.deepstack_mergers.len());
        for (layer, block) in self.blocks.iter().enumerate() {
            xs = block.forward(&xs, &cu_seqlens, &cos, &sin)?;
            if let Some(i) = self.deepstack_lookup[layer] {
                deepstack.push(self.deepstack_mergers[i].forward(&xs)?);
            }
        }
        Ok((self.merger.forward(&xs)?, deepstack))
    }
}
