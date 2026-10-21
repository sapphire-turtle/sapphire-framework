//! Images to patches, the image prompt, and its MRoPE positions. Pure; tested without a model.
//!
//! Ported from fastembed 7.1.1 (`models/qwen3.rs`), which follows qwen-vl-utils and the
//! `Qwen2VLImageProcessor`.

use image::DynamicImage;
use image::imageops::FilterType;
use serde::Deserialize;

/// `preprocessor_config.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct PreprocessorConfig {
    pub min_pixels: usize,
    pub max_pixels: usize,
    pub patch_size: usize,
    pub temporal_patch_size: usize,
    pub merge_size: usize,
    pub rescale_factor: f32,
    pub image_mean: [f32; 3],
    pub image_std: [f32; 3],
}

/// One image as the vision tower takes it.
#[derive(Debug)]
pub struct Patches {
    /// `[t·h·w, 3·T·P·P]`, row-major, in merge order.
    pub values: Vec<f32>,
    /// The grid of patches, `[t, h, w]`.
    pub grid: [usize; 3],
    /// The tokens it becomes in the text model: `t·h·w / merge²`.
    pub tokens: usize,
}

/// Python's `round()`: halves go to the even neighbour.
fn round_ties_to_even(value: f64) -> usize {
    let floor = value.floor();
    let frac = value - floor;
    if frac < 0.5 || (frac == 0.5 && (floor as i64) % 2 == 0) {
        floor as usize
    } else {
        (floor + 1.0) as usize
    }
}

/// The size to resize to: both sides multiples of `factor`, the area within the bounds and
/// the aspect ratio kept as far as those allow. `(height, width)`.
pub fn smart_resize(
    height: usize,
    width: usize,
    factor: usize,
    min_pixels: usize,
    max_pixels: usize,
) -> Result<(usize, usize), String> {
    if height == 0 || width == 0 {
        return Err("the image is empty".into());
    }
    if height.max(width) as f64 / height.min(width) as f64 > 200.0 {
        return Err("the image's aspect ratio is over 200".into());
    }
    let snap = |v: f64| (round_ties_to_even(v / factor as f64) * factor).max(factor);
    let (mut h, mut w) = (snap(height as f64), snap(width as f64));
    let area = (height * width) as f64;
    if h * w > max_pixels {
        let beta = (area / max_pixels as f64).sqrt();
        h = ((height as f64 / beta / factor as f64).floor() as usize * factor).max(factor);
        w = ((width as f64 / beta / factor as f64).floor() as usize * factor).max(factor);
    } else if h * w < min_pixels {
        let beta = (min_pixels as f64 / area).sqrt();
        h = ((height as f64 * beta / factor as f64).ceil() as usize * factor).max(factor);
        w = ((width as f64 * beta / factor as f64).ceil() as usize * factor).max(factor);
    }
    Ok((h, w))
}

/// `image` resized to at most `max_tokens` text-model tokens, normalized and cut into
/// patches.
pub fn patches(
    image: &DynamicImage,
    cfg: &PreprocessorConfig,
    max_tokens: usize,
) -> Result<Patches, String> {
    let (p, t, m) = (cfg.patch_size, cfg.temporal_patch_size, cfg.merge_size);
    if p == 0 || t == 0 || m == 0 {
        return Err("patch_size, temporal_patch_size and merge_size must be positive".into());
    }
    let rgb = image.to_rgb8();
    let (orig_w, orig_h) = rgb.dimensions();
    let factor = p * m;
    let max_pixels = cfg.max_pixels.min(max_tokens * factor * factor);
    let (h, w) = smart_resize(
        orig_h as usize,
        orig_w as usize,
        factor,
        cfg.min_pixels.min(max_pixels),
        max_pixels,
    )?;
    let resized = image::imageops::resize(&rgb, w as u32, h as u32, FilterType::CatmullRom);

    let (grid_h, grid_w) = (h / p, w / p);
    let mut values = Vec::with_capacity(grid_h * grid_w * 3 * t * p * p);
    // A still image fills every temporal slot of a patch with the same frame.
    for bh in 0..grid_h / m {
        for bw in 0..grid_w / m {
            for mh in 0..m {
                for mw in 0..m {
                    let (gh, gw) = (bh * m + mh, bw * m + mw);
                    for c in 0..3 {
                        for _ in 0..t {
                            for ph in 0..p {
                                for pw in 0..p {
                                    let pixel = resized
                                        .get_pixel((gw * p + pw) as u32, (gh * p + ph) as u32);
                                    let v = pixel.0[c] as f32 * cfg.rescale_factor;
                                    values.push((v - cfg.image_mean[c]) / cfg.image_std[c]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(Patches {
        values,
        grid: [1, grid_h, grid_w],
        tokens: grid_h * grid_w / (m * m),
    })
}

/// The image placeholder in the prompt; [`image_prompt`] repeats it once per token.
pub const IMAGE_PAD: &str = "<|image_pad|>";

/// The official template with an image in place of the text, `tokens` placeholders long.
pub fn image_prompt(tokens: usize) -> String {
    format!(
        "<|im_start|>system\nRepresent the user's input.<|im_end|>\n<|im_start|>user\n<|vision_start|>{}<|vision_end|><|im_end|>\n<|im_start|>assistant\n",
        IMAGE_PAD.repeat(tokens)
    )
}

/// The MRoPE positions `[3][len]` of a prompt of `len` tokens whose image tokens are
/// `start..start + t·h·w` on a `[t, h, w]` grid of merged tokens.
///
/// Text before the image counts up on all three axes; image tokens take `start` plus their
/// frame, row and column; text after it continues from one past the largest position used.
pub fn image_positions(len: usize, start: usize, grid: [usize; 3]) -> Vec<[u32; 3]> {
    let [t, h, w] = grid;
    let end = start + t * h * w;
    let mut out = Vec::with_capacity(len);
    for pos in 0..start {
        out.push([pos as u32; 3]);
    }
    for ti in 0..t {
        for hi in 0..h {
            for wi in 0..w {
                out.push([
                    (start + ti) as u32,
                    (start + hi) as u32,
                    (start + wi) as u32,
                ]);
            }
        }
    }
    let next = start + t.max(h).max(w);
    for offset in 0..len.saturating_sub(end) {
        out.push([(next + offset) as u32; 3]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PreprocessorConfig {
        PreprocessorConfig {
            min_pixels: 4096,
            max_pixels: 1_310_720,
            patch_size: 16,
            temporal_patch_size: 2,
            merge_size: 2,
            rescale_factor: 1.0 / 255.0,
            image_mean: [0.5; 3],
            image_std: [0.5; 3],
        }
    }

    #[test]
    fn rounds_half_to_even_like_python() {
        assert_eq!(round_ties_to_even(12.5), 12);
        assert_eq!(round_ties_to_even(13.5), 14);
        assert_eq!(round_ties_to_even(9.5625), 10);
    }

    #[test]
    fn resizing_snaps_to_the_factor_and_respects_the_bounds() {
        assert_eq!(smart_resize(100, 200, 32, 4096, 1 << 20), Ok((96, 192)));
        let (h, w) = smart_resize(4000, 3000, 32, 4096, 1024 * 32 * 32).unwrap();
        assert!(h * w <= 1024 * 32 * 32, "{h}x{w}");
        assert_eq!((h % 32, w % 32), (0, 0));
        // Too small grows to the minimum.
        let (h, w) = smart_resize(10, 10, 32, 4096, 1 << 20).unwrap();
        assert!(h * w >= 4096);
        assert!(smart_resize(1, 300, 32, 4096, 1 << 20).is_err());
        assert!(smart_resize(0, 3, 32, 4096, 1 << 20).is_err());
    }

    #[test]
    fn patches_follow_the_token_budget_and_merge_order() {
        let mut img = image::RgbImage::new(640, 320);
        // The left half is black, the right half white.
        for (x, _, px) in img.enumerate_pixels_mut() {
            *px = image::Rgb(if x < 320 { [0; 3] } else { [255; 3] });
        }
        let img = DynamicImage::ImageRgb8(img);
        let p = patches(&img, &config(), 64).unwrap();
        assert!(p.tokens <= 64, "{} tokens", p.tokens);
        let [t, h, w] = p.grid;
        assert_eq!(t, 1);
        assert_eq!(p.tokens, h * w / 4);
        let patch = 3 * 2 * 16 * 16;
        assert_eq!(p.values.len(), h * w * patch);
        // The first patch is the top-left corner: black, normalized to -1.
        assert!(p.values[..patch].iter().all(|&v| (v + 1.0).abs() < 1e-6));
        // The second is its right neighbour in the same 2×2 block, still black; the last
        // patch is the bottom-right corner: white, +1.
        assert!(
            p.values[patch..2 * patch]
                .iter()
                .all(|&v| (v + 1.0).abs() < 1e-6)
        );
        assert!(
            p.values[p.values.len() - patch..]
                .iter()
                .all(|&v| (v - 1.0).abs() < 1e-6)
        );
    }

    #[test]
    fn the_prompt_holds_one_placeholder_per_token() {
        let prompt = image_prompt(3);
        assert_eq!(prompt.matches(IMAGE_PAD).count(), 3);
        assert!(prompt.contains("<|vision_start|><|image_pad|>"));
        assert!(prompt.ends_with("<|im_end|>\n<|im_start|>assistant\n"));
    }

    #[test]
    fn positions_count_the_image_by_its_grid() {
        // 2 text tokens, a 1×2×3 image, 2 text tokens.
        let pos = image_positions(10, 2, [1, 2, 3]);
        assert_eq!(pos.len(), 10);
        assert_eq!(pos[0], [0, 0, 0]);
        assert_eq!(pos[1], [1, 1, 1]);
        assert_eq!(pos[2], [2, 2, 2]);
        assert_eq!(pos[4], [2, 2, 4]);
        assert_eq!(pos[7], [2, 3, 4]);
        // After the image: one past start + max(t, h, w) - 1.
        assert_eq!(pos[8], [5, 5, 5]);
        assert_eq!(pos[9], [6, 6, 6]);
    }
}
