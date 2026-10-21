//! Linear layers and the token embedding, held at a [`QType`] chosen while loading.

use candle_core::quantized::{GgmlDType, QMatMul, QTensor};
use candle_core::{DType, Module, Result, Tensor};
use candle_nn::VarBuilder;
use sapphire_bridge_api::QType;

/// The storage `qtype` gives a weight whose rows are `width` values long.
///
/// A quantized type packs each row in blocks; a row that does not divide into them falls
/// back to the next wider type: Q4_K → Q8_0 → F16.
pub fn storage_for(qtype: QType, width: usize) -> QType {
    let fits = |dtype: GgmlDType| width.is_multiple_of(dtype.block_size());
    match qtype {
        QType::Q4K if fits(GgmlDType::Q4K) => QType::Q4K,
        QType::Q4K | QType::Q8_0 if fits(GgmlDType::Q8_0) => QType::Q8_0,
        QType::F32 => QType::F32,
        _ => QType::F16,
    }
}

/// `weight` (`[out, in]`, f32) held as `qtype`, or the type it falls back to.
fn hold(weight: Tensor, qtype: QType) -> Result<QMatMul> {
    let width = weight.dim(1)?;
    Ok(match storage_for(qtype, width) {
        QType::F32 => QMatMul::Tensor(weight),
        QType::F16 => QMatMul::TensorF16(weight.to_dtype(DType::F16)?),
        QType::Q8_0 => QMatMul::from_qtensor(QTensor::quantize(&weight, GgmlDType::Q8_0)?)?,
        QType::Q4K => QMatMul::from_qtensor(QTensor::quantize(&weight, GgmlDType::Q4K)?)?,
    })
}

/// A linear layer: `x · Wᵀ + b`, the weight at a [`QType`] and the bias at f32.
pub struct QLinear {
    weight: QMatMul,
    bias: Option<Tensor>,
}

impl QLinear {
    /// Load `weight` (and `bias`, when `bias`) from `vb`, which must yield f32.
    pub fn new(
        in_dim: usize,
        out_dim: usize,
        bias: bool,
        qtype: QType,
        vb: VarBuilder,
    ) -> Result<Self> {
        let bias = if bias {
            Some(vb.get(out_dim, "bias")?)
        } else {
            None
        };
        Self::from_weight(vb.get((out_dim, in_dim), "weight")?, bias, qtype)
    }

    /// A layer over an f32 `weight` (`[out, in]`) already in hand.
    pub fn from_weight(weight: Tensor, bias: Option<Tensor>, qtype: QType) -> Result<Self> {
        Ok(Self {
            weight: hold(weight, qtype)?,
            bias,
        })
    }
}

impl Module for QLinear {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        // The quantized kernel reads its input as one contiguous slice.
        let ys = self.weight.forward(&xs.contiguous()?)?;
        match &self.bias {
            Some(bias) => ys.broadcast_add(bias),
            None => Ok(ys),
        }
    }
}

/// A token embedding held at a [`QType`]; rows come out at f32.
pub struct QEmbedding {
    table: QMatMul,
}

impl QEmbedding {
    /// Load `weight` (`[vocab, hidden]`) from `vb`, which must yield f32.
    pub fn new(vocab: usize, hidden: usize, qtype: QType, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            table: hold(vb.get((vocab, hidden), "weight")?, qtype)?,
        })
    }

    /// The rows of `ids` (`[B, T]` → `[B, T, hidden]`).
    pub fn forward(&self, ids: &Tensor) -> Result<Tensor> {
        self.table.embedding(ids)?.to_dtype(DType::F32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::Device;
    use std::collections::HashMap;

    #[test]
    fn rows_that_do_not_fit_a_block_fall_back() {
        assert_eq!(storage_for(QType::Q4K, 2048), QType::Q4K);
        assert_eq!(storage_for(QType::Q4K, 1152), QType::Q8_0);
        assert_eq!(storage_for(QType::Q4K, 4304), QType::F16);
        assert_eq!(storage_for(QType::Q8_0, 4304), QType::F16);
        assert_eq!(storage_for(QType::Q8_0, 1152), QType::Q8_0);
        assert_eq!(storage_for(QType::F16, 2048), QType::F16);
        assert_eq!(storage_for(QType::F32, 4304), QType::F32);
    }

    fn weights(out_dim: usize, in_dim: usize) -> VarBuilder<'static> {
        let values: Vec<f32> = (0..out_dim * in_dim)
            .map(|i| ((i * 7919) % 101) as f32 / 101.0 - 0.5)
            .collect();
        let weight = Tensor::from_vec(values, (out_dim, in_dim), &Device::Cpu).unwrap();
        let bias = Tensor::ones(out_dim, DType::F32, &Device::Cpu).unwrap();
        let map = HashMap::from([("weight".to_owned(), weight), ("bias".to_owned(), bias)]);
        VarBuilder::from_tensors(map, DType::F32, &Device::Cpu)
    }

    #[test]
    fn every_qtype_stays_close_to_f32() {
        let (out_dim, in_dim) = (8, 512);
        // A fixed input, so the bounds below are not luck.
        let values: Vec<f32> = (0..2 * 3 * in_dim)
            .map(|i| ((i * 104729) % 997) as f32 / 498.5 - 1.0)
            .collect();
        let xs = Tensor::from_vec(values, (2, 3, in_dim), &Device::Cpu).unwrap();
        let out = |qtype| {
            QLinear::new(in_dim, out_dim, true, qtype, weights(out_dim, in_dim))
                .unwrap()
                .forward(&xs)
                .unwrap()
                .flatten_all()
                .unwrap()
                .to_vec1::<f32>()
                .unwrap()
        };
        let reference = out(QType::F32);
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        for (qtype, min_cos) in [
            (QType::F16, 0.9999),
            (QType::Q8_0, 0.999),
            (QType::Q4K, 0.98),
        ] {
            let ys = out(qtype);
            assert_eq!(ys.len(), 2 * 3 * out_dim);
            let cos = ys.iter().zip(&reference).map(|(a, b)| a * b).sum::<f32>()
                / (norm(&ys) * norm(&reference));
            assert!(cos > min_cos, "{qtype}: cosine {cos}");
        }
    }

    #[test]
    fn embedding_rows_come_out_at_f32() {
        for qtype in QType::ALL {
            let table = QEmbedding::new(4, 256, qtype, weights(4, 256)).unwrap();
            let ids = Tensor::new(&[[0u32, 3]], &Device::Cpu).unwrap();
            let rows = table.forward(&ids).unwrap();
            assert_eq!(rows.dims(), &[1, 2, 256]);
            assert_eq!(rows.dtype(), DType::F32);
        }
    }
}
