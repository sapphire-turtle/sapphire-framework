//! Qwen3-VL-Embedding on candle, with the weights quantized while loading.
//!
//! Ported from fastembed 7.1.1 (Apache-2.0), whose model builds every projection as a dense
//! `candle_nn::Linear`: here each one is a [`qlinear::QLinear`] at the device's
//! [`QType`](sapphire_bridge_api::QType).

pub mod preprocess;
pub mod qlinear;
pub mod text;
pub mod vision;
