//! [`Tint`]: a colour with no lightness of its own — a swatch of the theme's, or a hue and a
//! chroma — and ways to pick one at random or from a name.

use super::Swatch;

use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

/// A colour with no lightness of its own: where it is used decides that.
#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Tint {
    /// One of the theme's, by name: it follows the theme.
    Swatch(Swatch),
    /// An OKLCH chroma and hue (in degrees), as the disk of a
    /// [`TintPicker`](super::TintPicker) gives them.
    Custom { chroma: f32, hue: f32 },
}

impl Tint {
    /// The most chroma a tint may have: the rim of the picker's disk.
    pub const MAX_CHROMA: f32 = 0.25;

    /// The point of the disk that `u1` and `u2` (each in `[0, 1)`) pick, evenly over its
    /// area: chroma `√u1 · MAX_CHROMA`, hue `360 · u2`. Vivid tints are as common as the
    /// rim is long, so greys are rare.
    pub fn from_unit(u1: f32, u2: f32) -> Tint {
        let unit = |u: f32| u.clamp(0.0, 1.0 - f32::EPSILON);
        Tint::Custom {
            chroma: unit(u1).sqrt() * Tint::MAX_CHROMA,
            hue: unit(u2) * 360.0,
        }
    }

    /// The tint `bytes` hash to — a label's name, say. The hash (FNV-1a, then
    /// MurmurHash3's finaliser) is fixed, so the same bytes give the same tint on every
    /// device and every version.
    pub fn from_bytes(bytes: &[u8]) -> Tint {
        Tint::from_u64(fmix64(fnv1a(bytes)))
    }

    /// A tint at random, evenly over the disk.
    pub fn random() -> Tint {
        static DRAWS: AtomicU64 = AtomicU64::new(0);
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(DRAWS.fetch_add(1, Ordering::Relaxed));
        Tint::from_u64(hasher.finish())
    }

    /// The high 32 bits pick the radius, the low 32 the angle.
    fn from_u64(x: u64) -> Tint {
        let unit = |bits: u64| (bits as f64 / (1u64 << 32) as f64) as f32;
        Tint::from_unit(unit(x >> 32), unit(x & 0xffff_ffff))
    }
}

/// MurmurHash3's 64-bit finaliser. FNV-1a alone leaves the high bits of short, similar
/// inputs ("label1", "label2", …) nearly the same, which bunches their tints together.
fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    k ^ (k >> 33)
}

/// FNV-1a, 64 bits.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(t: Tint) -> (f32, f32) {
        match t {
            Tint::Custom { chroma, hue } => (chroma, hue),
            other => panic!("{other:?} is not Custom"),
        }
    }

    #[test]
    fn from_unit_maps_radius_and_angle() {
        assert_eq!(custom(Tint::from_unit(0.0, 0.5)).0, 0.0);
        let (chroma, hue) = custom(Tint::from_unit(0.999_999, 0.25));
        assert!((chroma - Tint::MAX_CHROMA).abs() < 1e-3, "{chroma}");
        assert!((hue - 90.0).abs() < 0.1, "{hue}");
    }

    #[test]
    fn from_unit_is_area_uniform() {
        let n = 100;
        let mut inner = 0;
        for i in 0..n {
            for j in 0..n {
                let u1 = (i as f32 + 0.5) / n as f32;
                let u2 = (j as f32 + 0.5) / n as f32;
                if custom(Tint::from_unit(u1, u2)).0 <= Tint::MAX_CHROMA / 2.0 {
                    inner += 1;
                }
            }
        }
        let share = inner as f32 / (n * n) as f32;
        assert!((0.24..=0.26).contains(&share), "{share}");
    }

    #[test]
    fn fnv1a_known_values() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn from_bytes_is_fixed() {
        let (chroma, hue) = custom(Tint::from_bytes(b"bug"));
        assert!((chroma - PINNED_CHROMA).abs() < 1e-6, "{chroma}");
        assert!((hue - PINNED_HUE).abs() < 1e-4, "{hue}");
    }

    /// `from_bytes(b"bug")` as first computed: a change here changes every label's colour.
    const PINNED_CHROMA: f32 = 0.238_907_53;
    const PINNED_HUE: f32 = 331.028_96;

    #[test]
    fn similar_names_spread_over_the_disk() {
        // Evenly over the area, 9% fall within 0.3 of the radius.
        let inner = (0..2000)
            .map(|i| custom(Tint::from_bytes(format!("label{i}").as_bytes())).0)
            .filter(|&c| c < 0.3 * Tint::MAX_CHROMA)
            .count();
        let share = inner as f32 / 2000.0;
        assert!((0.06..=0.12).contains(&share), "{share}");
    }

    #[test]
    fn from_bytes_differs() {
        assert_ne!(Tint::from_bytes(b"bug"), Tint::from_bytes(b"feature"));
    }

    #[test]
    fn random_is_in_range_and_varies() {
        let draws: Vec<(f32, f32)> = (0..100).map(|_| custom(Tint::random())).collect();
        for &(chroma, hue) in &draws {
            assert!((0.0..=Tint::MAX_CHROMA).contains(&chroma), "{chroma}");
            assert!((0.0..360.0).contains(&hue), "{hue}");
        }
        assert!(draws.iter().any(|d| *d != draws[0]));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn tint_round_trips() {
        for t in [
            Tint::Swatch(Swatch::Red),
            Tint::Custom {
                chroma: 0.2,
                hue: 140.0,
            },
        ] {
            let json = serde_json::to_string(&t).unwrap();
            assert_eq!(serde_json::from_str::<Tint>(&json).unwrap(), t, "{json}");
        }
    }
}
