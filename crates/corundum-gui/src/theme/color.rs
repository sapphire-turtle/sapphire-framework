//! OKLCH, converted to sRGB by hand (Björn Ottosson's OKLab). The way back and the WCAG
//! contrast ratio are for the tests, which hold the themes to their contrast targets.

use egui::Color32;

/// A colour in OKLCH: lightness (0 black – 1 white), chroma (0 grey – about 0.37), hue in
/// degrees.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

/// `c` in sRGB. A colour sRGB cannot show keeps its lightness and hue and loses chroma
/// until it fits.
pub(crate) fn to_color32(c: Oklch) -> Color32 {
    let mut rgb = linear(c);
    if !in_gamut(rgb) {
        let (mut lo, mut hi) = (0.0, c.c);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(linear(Oklch { c: mid, ..c })) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = linear(Oklch { c: lo, ..c });
    }
    let [r, g, b] = rgb.map(|x| (encode(x.clamp(0.0, 1.0)) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// `c` in OKLCH; its alpha is ignored.
#[cfg(test)]
pub(crate) fn from_color32(c: Color32) -> Oklch {
    let [r, g, b] = [c.r(), c.g(), c.b()].map(|x| decode(x as f32 / 255.0));
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let [l, m, s] = [l, m, s].map(f32::cbrt);
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    Oklch {
        l: lightness,
        c: a.hypot(b),
        h: b.atan2(a).to_degrees().rem_euclid(360.0),
    }
}

/// The WCAG 2 contrast ratio of `a` and `b`, from 1 (the same) to 21 (black on white).
#[cfg(test)]
pub(crate) fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
fn luminance(c: Color32) -> f32 {
    let [r, g, b] = [c.r(), c.g(), c.b()].map(|x| decode(x as f32 / 255.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// `c` in linear sRGB, possibly outside `[0, 1]`.
fn linear(c: Oklch) -> [f32; 3] {
    let h = c.h.rem_euclid(360.0).to_radians();
    let (a, b) = (c.c * h.cos(), c.c * h.sin());
    let l = c.l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m = c.l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s = c.l - 0.089_484_18 * a - 1.291_485_5 * b;
    let [l, m, s] = [l, m, s].map(|x| x * x * x);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|x| (-1e-4..=1.0 + 1e-4).contains(x))
}

/// The sRGB transfer function, linear to encoded.
fn encode(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Encoded sRGB to linear.
#[cfg(test)]
fn decode(x: f32) -> f32 {
    if x <= 0.040_45 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Color32, b: Color32) -> bool {
        a.to_array()
            .iter()
            .zip(b.to_array())
            .all(|(x, y)| x.abs_diff(y) <= 1)
    }

    #[test]
    fn round_trips_white_black_and_primaries() {
        for c in [
            Color32::WHITE,
            Color32::BLACK,
            Color32::from_rgb(255, 0, 0),
            Color32::from_rgb(0, 255, 0),
            Color32::from_rgb(0, 0, 255),
        ] {
            let back = to_color32(from_color32(c));
            assert!(close(back, c), "{c:?} came back as {back:?}");
        }
    }

    #[test]
    fn white_is_l1_black_is_l0() {
        assert!((from_color32(Color32::WHITE).l - 1.0).abs() < 0.001);
        assert!(from_color32(Color32::BLACK).l.abs() < 0.001);
    }

    #[test]
    fn out_of_gamut_keeps_lightness_and_hue() {
        let out = from_color32(to_color32(Oklch {
            l: 0.3,
            c: 0.4,
            h: 95.0,
        }));
        assert!((out.l - 0.3).abs() < 0.01, "{out:?}");
        assert!((out.h - 95.0).abs() < 2.0, "{out:?}");
    }

    #[test]
    fn contrast_extremes() {
        let white = Color32::WHITE;
        let black = Color32::BLACK;
        assert!((contrast_ratio(white, black) - 21.0).abs() < 0.05);
        assert_eq!(contrast_ratio(black, white), contrast_ratio(white, black));
        let grey = Color32::from_gray(120);
        assert!((contrast_ratio(grey, grey) - 1.0).abs() < 1e-6);
    }
}
