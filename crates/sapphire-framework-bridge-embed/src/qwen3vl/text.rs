//! The Qwen3 text model (the language tower of Qwen3-VL), at f32 activations.
//!
//! Ported from fastembed 7.1.1 (`models/qwen3.rs`), with every projection behind
//! [`QLinear`] and the token embedding behind [`QEmbedding`].

use candle_core::{D, DType, Device, Module, Result, Tensor};
use candle_nn::{Activation, VarBuilder};
use sapphire_bridge_api::QType;
use serde::Deserialize;

use super::qlinear::{QEmbedding, QLinear};

fn default_true() -> bool {
    true
}

/// The text model's `config.json` (or a Qwen3-VL config's `text_config`).
#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub attention_bias: bool,
    pub head_dim: Option<usize>,
    pub hidden_act: Activation,
    pub hidden_size: usize,
    pub intermediate_size: usize,
    pub num_attention_heads: usize,
    pub num_hidden_layers: usize,
    pub num_key_value_heads: usize,
    pub rms_norm_eps: f64,
    pub rope_scaling: Option<serde_json::Value>,
    pub rope_theta: f64,
    #[serde(default = "default_true")]
    pub use_cache: bool,
    pub vocab_size: usize,
}

impl Config {
    pub fn head_dim(&self) -> usize {
        self.head_dim
            .unwrap_or(self.hidden_size / self.num_attention_heads)
    }

    fn num_kv_groups(&self) -> usize {
        self.num_attention_heads / self.num_key_value_heads
    }
}

struct RmsNorm {
    weight: Tensor,
    eps: f64,
}

impl RmsNorm {
    fn new(dim: usize, eps: f64, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            weight: vb.get((dim,), "weight")?,
            eps,
        })
    }
}

impl Module for RmsNorm {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let var = xs.powf(2.0)?.mean_keepdim(D::Minus1)?;
        let inv_rms = (var + self.eps)?.sqrt()?.recip()?;
        xs.broadcast_mul(&inv_rms)?.broadcast_mul(&self.weight)
    }
}

struct Mlp {
    gate_proj: QLinear,
    up_proj: QLinear,
    down_proj: QLinear,
    act_fn: Activation,
}

impl Mlp {
    fn new(cfg: &Config, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let (h, i) = (cfg.hidden_size, cfg.intermediate_size);
        Ok(Self {
            gate_proj: QLinear::new(h, i, false, qtype, vb.pp("gate_proj"))?,
            up_proj: QLinear::new(h, i, false, qtype, vb.pp("up_proj"))?,
            down_proj: QLinear::new(i, h, false, qtype, vb.pp("down_proj"))?,
            act_fn: cfg.hidden_act,
        })
    }
}

impl Module for Mlp {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let lhs = xs.apply(&self.gate_proj)?.apply(&self.act_fn)?;
        let rhs = xs.apply(&self.up_proj)?;
        (lhs * rhs)?.apply(&self.down_proj)
    }
}

/// RoPE, with Qwen3-VL's interleaved multimodal sections when positions are `[3, B, T]`.
struct RotaryEmbedding {
    inv_freq: Vec<f32>,
    mrope_interleaved: bool,
    mrope_section: [usize; 3],
}

impl RotaryEmbedding {
    fn new(cfg: &Config) -> Self {
        let dim = cfg.head_dim();
        let ln_base = (cfg.rope_theta as f32).ln();
        let inv_freq = (0..dim)
            .step_by(2)
            .map(|i| (-ln_base * i as f32 / dim as f32).exp())
            .collect();

        let mut mrope_interleaved = false;
        let mut mrope_section = [24usize, 20, 20];
        if let Some(rope_scaling) = cfg.rope_scaling.as_ref() {
            if let Some(v) = rope_scaling
                .get("mrope_interleaved")
                .and_then(|v| v.as_bool())
            {
                mrope_interleaved = v;
            }
            if let Some(arr) = rope_scaling.get("mrope_section").and_then(|v| v.as_array())
                && arr.len() == 3
            {
                for (slot, v) in mrope_section.iter_mut().zip(arr) {
                    *slot = v.as_u64().map_or(*slot, |v| v as usize);
                }
            }
        }
        Self {
            inv_freq,
            mrope_interleaved,
            mrope_section,
        }
    }

    /// `(cos, sin)`, each `[B, T, head_dim]`, for positions `[B, T]` or `[3, B, T]`.
    fn forward(&self, position_ids: &Tensor, device: &Device) -> Result<(Tensor, Tensor)> {
        let d2 = self.inv_freq.len();
        let (sections, b, t) = match position_ids.rank() {
            2 => {
                let (b, t) = position_ids.dims2()?;
                (position_ids.unsqueeze(0)?, b, t)
            }
            _ => {
                let (dims, b, t) = position_ids.dims3()?;
                if dims != 3 {
                    candle_core::bail!("MRoPE position ids must be [3, B, T]");
                }
                (position_ids.clone(), b, t)
            }
        };
        let pos = sections.to_vec3::<u32>()?;
        let mut freqs = vec![0f32; b * t * d2];
        for batch in 0..b {
            for tok in 0..t {
                let base = (batch * t + tok) * d2;
                let temporal = pos[0][batch][tok] as f32;
                for (i, inv) in self.inv_freq.iter().enumerate() {
                    freqs[base + i] = temporal * inv;
                }
                if self.mrope_interleaved && pos.len() == 3 {
                    // Height and width take every third frequency from 1 and 2 on.
                    for (dim, axis) in pos.iter().enumerate().skip(1) {
                        let p = axis[batch][tok] as f32;
                        let limit = (self.mrope_section[dim] * 3).min(d2);
                        for i in (dim..limit).step_by(3) {
                            freqs[base + i] = p * self.inv_freq[i];
                        }
                    }
                }
            }
        }
        let freqs = Tensor::from_vec(freqs, (b, t, d2), device)?;
        let emb = Tensor::cat(&[&freqs, &freqs], 2)?;
        Ok((emb.cos()?, emb.sin()?))
    }
}

fn rotate_half(x: &Tensor) -> Result<Tensor> {
    let half = x.dim(D::Minus1)? / 2;
    let x1 = x.narrow(D::Minus1, 0, half)?;
    let x2 = x.narrow(D::Minus1, half, half)?;
    Tensor::cat(&[&x2.neg()?, &x1], D::Minus1)
}

/// `q`, `k`: `[B, H, T, D]`; `cos`, `sin`: `[B, T, D]`.
fn apply_rotary(q: &Tensor, k: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<(Tensor, Tensor)> {
    let cos = cos.unsqueeze(1)?;
    let sin = sin.unsqueeze(1)?;
    let q = (q.broadcast_mul(&cos)? + rotate_half(q)?.broadcast_mul(&sin)?)?;
    let k = (k.broadcast_mul(&cos)? + rotate_half(k)?.broadcast_mul(&sin)?)?;
    Ok((q, k))
}

/// `[B, Nkv, T, D]` → `[B, Nkv * n_rep, T, D]`.
fn repeat_kv(x: Tensor, n_rep: usize) -> Result<Tensor> {
    if n_rep == 1 {
        return Ok(x);
    }
    let (b, n_kv, t, d) = x.dims4()?;
    x.unsqueeze(2)?
        .broadcast_as((b, n_kv, n_rep, t, d))?
        .reshape((b, n_kv * n_rep, t, d))
}

struct Attention {
    q_proj: QLinear,
    k_proj: QLinear,
    v_proj: QLinear,
    o_proj: QLinear,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    num_heads: usize,
    num_kv_heads: usize,
    num_kv_groups: usize,
    head_dim: usize,
    scaling: f64,
}

impl Attention {
    fn new(cfg: &Config, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let head_dim = cfg.head_dim();
        let (heads, kv_heads) = (cfg.num_attention_heads, cfg.num_key_value_heads);
        if !heads.is_multiple_of(kv_heads) {
            candle_core::bail!("num_attention_heads must be a multiple of num_key_value_heads");
        }
        let (h, q_out, kv_out) = (cfg.hidden_size, heads * head_dim, kv_heads * head_dim);
        let bias = cfg.attention_bias;
        Ok(Self {
            q_proj: QLinear::new(h, q_out, bias, qtype, vb.pp("q_proj"))?,
            k_proj: QLinear::new(h, kv_out, bias, qtype, vb.pp("k_proj"))?,
            v_proj: QLinear::new(h, kv_out, bias, qtype, vb.pp("v_proj"))?,
            o_proj: QLinear::new(q_out, h, bias, qtype, vb.pp("o_proj"))?,
            q_norm: RmsNorm::new(head_dim, cfg.rms_norm_eps, vb.pp("q_norm"))?,
            k_norm: RmsNorm::new(head_dim, cfg.rms_norm_eps, vb.pp("k_norm"))?,
            num_heads: heads,
            num_kv_heads: kv_heads,
            num_kv_groups: cfg.num_kv_groups(),
            head_dim,
            scaling: (head_dim as f64).powf(-0.5),
        })
    }

    /// `xs`: `[B, T, H]`; `mask`: additive `[B, 1, T, T]`.
    fn forward(&self, xs: &Tensor, rope: (&Tensor, &Tensor), mask: &Tensor) -> Result<Tensor> {
        let (b, t, _) = xs.dims3()?;
        let d = self.head_dim;
        let q = xs
            .apply(&self.q_proj)?
            .reshape((b, t, self.num_heads, d))?
            .apply(&self.q_norm)?
            .transpose(1, 2)?;
        let k = xs
            .apply(&self.k_proj)?
            .reshape((b, t, self.num_kv_heads, d))?
            .apply(&self.k_norm)?
            .transpose(1, 2)?;
        let v = xs
            .apply(&self.v_proj)?
            .reshape((b, t, self.num_kv_heads, d))?
            .transpose(1, 2)?;
        let (q, k) = apply_rotary(&q, &k, rope.0, rope.1)?;
        let k = repeat_kv(k, self.num_kv_groups)?;
        let v = repeat_kv(v, self.num_kv_groups)?;

        let attn = (q.matmul(&k.transpose(2, 3)?)? * self.scaling)?.broadcast_add(mask)?;
        let attn = candle_nn::ops::softmax_last_dim(&attn)?;
        attn.matmul(&v)?
            .transpose(1, 2)?
            .reshape((b, t, self.num_heads * d))?
            .apply(&self.o_proj)
    }
}

struct DecoderLayer {
    self_attn: Attention,
    mlp: Mlp,
    input_layernorm: RmsNorm,
    post_attention_layernorm: RmsNorm,
}

impl DecoderLayer {
    fn new(cfg: &Config, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let (h, eps) = (cfg.hidden_size, cfg.rms_norm_eps);
        Ok(Self {
            self_attn: Attention::new(cfg, qtype, vb.pp("self_attn"))?,
            mlp: Mlp::new(cfg, qtype, vb.pp("mlp"))?,
            input_layernorm: RmsNorm::new(h, eps, vb.pp("input_layernorm"))?,
            post_attention_layernorm: RmsNorm::new(h, eps, vb.pp("post_attention_layernorm"))?,
        })
    }

    fn forward(&self, xs: &Tensor, mask: &Tensor, rope: (&Tensor, &Tensor)) -> Result<Tensor> {
        let attn = self
            .self_attn
            .forward(&xs.apply(&self.input_layernorm)?, rope, mask)?;
        let xs = (xs + attn)?;
        let mlp = xs.apply(&self.post_attention_layernorm)?.apply(&self.mlp)?;
        xs + mlp
    }
}

/// The text model, from token embeddings to the final norm.
pub struct TextModel {
    embed_tokens: QEmbedding,
    layers: Vec<DecoderLayer>,
    norm: RmsNorm,
    rotary: RotaryEmbedding,
    device: Device,
}

impl TextModel {
    /// Load from `vb` (f32, at the model's prefix), each projection held as `qtype`.
    pub fn new(cfg: &Config, qtype: QType, vb: VarBuilder) -> Result<Self> {
        let embed_tokens = QEmbedding::new(
            cfg.vocab_size,
            cfg.hidden_size,
            qtype,
            vb.pp("embed_tokens"),
        )?;
        let layers = (0..cfg.num_hidden_layers)
            .map(|i| DecoderLayer::new(cfg, qtype, vb.pp(format!("layers.{i}"))))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            embed_tokens,
            layers,
            norm: RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("norm"))?,
            rotary: RotaryEmbedding::new(cfg),
            device: vb.device().clone(),
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// `[B, T]` token ids → `[B, T, H]` embeddings.
    pub fn embed_tokens(&self, input_ids: &Tensor) -> Result<Tensor> {
        self.embed_tokens.forward(input_ids)
    }

    /// The hidden states `[B, T, H]` after the final norm.
    ///
    /// `mask` is additive `[B, 1, T, T]`; `positions` is `[B, T]` or `[3, B, T]` (MRoPE).
    /// `deepstack[i]` (`[B, T, H]`) is added after layer `i`.
    pub fn forward(
        &self,
        inputs_embeds: &Tensor,
        mask: &Tensor,
        positions: &Tensor,
        deepstack: &[Tensor],
    ) -> Result<Tensor> {
        let (cos, sin) = self.rotary.forward(positions, &self.device)?;
        let mut hs = inputs_embeds.clone();
        for (i, layer) in self.layers.iter().enumerate() {
            hs = layer.forward(&hs, mask, (&cos, &sin))?;
            if let Some(add) = deepstack.get(i) {
                hs = (hs + add)?;
            }
        }
        hs.apply(&self.norm)
    }
}

/// The additive mask `[B, 1, T, T]`: causal, and `-1e4` on padding (`mask_2d` is 1 or 0).
pub fn attention_mask(mask_2d: &Tensor) -> Result<Tensor> {
    let (b, t) = mask_2d.dims2()?;
    let device = mask_2d.device();
    let mask_value = -1e4f32;
    let causal: Vec<f32> = (0..t)
        .flat_map(|i| (0..t).map(move |j| if j > i { mask_value } else { 0.0 }))
        .collect();
    let causal = Tensor::from_vec(causal, (1, 1, t, t), device)?;
    let pad =
        ((mask_2d.to_dtype(DType::F32)? - 1.0)? * -(mask_value as f64))?.reshape((b, 1, 1, t))?;
    causal.broadcast_add(&pad)
}

/// Each row L2-normalized (`[B, H]`).
pub fn l2_normalize(xs: &Tensor) -> Result<Tensor> {
    let norm = (xs.sqr()?.sum_keepdim(1)? + 1e-12)?.sqrt()?;
    xs.broadcast_div(&norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mask_is_causal_and_hides_padding() {
        let mask_2d = Tensor::new(&[[0f32, 1., 1.]], &Device::Cpu).unwrap();
        let mask = attention_mask(&mask_2d)
            .unwrap()
            .squeeze(0)
            .unwrap()
            .squeeze(0)
            .unwrap()
            .to_vec2::<f32>()
            .unwrap();
        // Row 2 sees columns 1 and 2, not the padding at 0.
        assert_eq!(mask[2], vec![-1e4, 0.0, 0.0]);
        // Row 1 sees column 1; column 2 is in its future.
        assert_eq!(mask[1], vec![-1e4, 0.0, -1e4]);
    }

    fn tiny_config(interleaved: bool) -> Config {
        serde_json::from_value(serde_json::json!({
            "attention_bias": false,
            "head_dim": 16,
            "hidden_act": "silu",
            "hidden_size": 64,
            "intermediate_size": 128,
            "num_attention_heads": 4,
            "num_hidden_layers": 1,
            "num_key_value_heads": 2,
            "rms_norm_eps": 1e-6,
            "rope_scaling": {"rope_type": "default", "mrope_section": [4, 2, 2], "mrope_interleaved": interleaved},
            "rope_theta": 10000,
            "vocab_size": 1000
        }))
        .unwrap()
    }

    #[test]
    fn mrope_with_equal_sections_is_plain_rope() {
        let rotary = RotaryEmbedding::new(&tiny_config(true));
        let positions: Vec<u32> = (0..2).flat_map(|_| 0..9u32).collect();
        let plain = Tensor::from_vec(positions.clone(), (2, 9), &Device::Cpu).unwrap();
        let mrope = Tensor::from_vec(positions.repeat(3), (3, 2, 9), &Device::Cpu).unwrap();
        let (cos_a, sin_a) = rotary.forward(&plain, &Device::Cpu).unwrap();
        let (cos_b, sin_b) = rotary.forward(&mrope, &Device::Cpu).unwrap();
        let diff = |a: &Tensor, b: &Tensor| {
            (a - b)
                .unwrap()
                .abs()
                .unwrap()
                .flatten_all()
                .unwrap()
                .max(0)
                .unwrap()
                .to_scalar::<f32>()
                .unwrap()
        };
        assert!(diff(&cos_a, &cos_b) < 1e-6);
        assert!(diff(&sin_a, &sin_b) < 1e-6);
    }

    #[test]
    fn mrope_reads_the_spatial_sections_interleaved() {
        let rotary = RotaryEmbedding::new(&tiny_config(true));
        // One token: temporal 0, height 1, width 0.
        let pos = Tensor::from_vec(vec![0u32, 1, 0], (3, 1, 1), &Device::Cpu).unwrap();
        let (cos, _) = rotary.forward(&pos, &Device::Cpu).unwrap();
        let cos = cos.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        // Frequency 1 belongs to the height section (1, 4, ...): cos(1 · inv_freq[1]) < 1.
        assert!(cos[1] < 1.0);
        // Frequencies 0 and 2 are temporal and width, both at position 0.
        assert_eq!(cos[0], 1.0);
        assert_eq!(cos[2], 1.0);
    }
}
