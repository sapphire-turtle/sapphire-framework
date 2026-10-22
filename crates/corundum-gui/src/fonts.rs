//! A CJK fallback font from the operating system, so apps ship no font assets.
//!
//! egui's built-in fonts have no CJK glyphs. Rather than bundling a 10–20 MB font, this
//! finds one the OS already has — preferring families with Japanese glyph forms — and
//! registers it behind egui's defaults, so Latin text keeps egui's fonts and CJK falls
//! through to the system font. Nothing is redistributed: the font is read at runtime.

use std::path::PathBuf;
use std::sync::Arc;

/// Families tried in order, Japanese glyph forms first.
#[cfg(target_os = "windows")]
pub const PREFERRED_FAMILIES: &[&str] = &[
    "Yu Gothic UI",
    "Yu Gothic",
    "Meiryo UI",
    "Meiryo",
    "MS Gothic",
];
/// Families tried in order, Japanese glyph forms first.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub const PREFERRED_FAMILIES: &[&str] = &[
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Hiragino Kaku Gothic Pro",
];
/// Families tried in order, Japanese glyph forms first. Older Android had Droid Sans
/// Japanese, or only Droid Sans Fallback.
#[cfg(target_os = "android")]
pub const PREFERRED_FAMILIES: &[&str] = &[
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Droid Sans Japanese",
    "Droid Sans Fallback",
];
/// Families tried in order, Japanese glyph forms first.
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
)))]
pub const PREFERRED_FAMILIES: &[&str] = &[
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Source Han Sans JP",
    "IPAexGothic",
    "IPAGothic",
    "Takao Gothic",
    "VL Gothic",
];

/// Glyphs a usable fallback must have.
const PROBE: [char; 2] = ['あ', '漢'];

/// The key the font is registered under in egui's font definitions.
const FONT_KEY: &str = "system-cjk";

/// A font file found on this system.
#[derive(Clone)]
pub struct SystemFont {
    /// Its family name.
    pub family: String,
    /// The whole font file (a collection keeps all its faces).
    pub data: Vec<u8>,
    /// Which face of the file to use.
    pub index: u32,
    /// Its CSS-style weight (400 is regular).
    pub weight: u16,
}

impl std::fmt::Debug for SystemFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemFont")
            .field("family", &self.family)
            .field("bytes", &self.data.len())
            .field("index", &self.index)
            .field("weight", &self.weight)
            .finish()
    }
}

/// One face, reduced to what [`pick`] decides on.
struct Candidate {
    id_index: usize,
    families: Vec<String>,
    covers_cjk: bool,
    /// How far the face is from plain regular: |weight - 400|, plus large penalties for a
    /// non-normal style or stretch. Smaller is better.
    regularity: u32,
}

/// Distance of a face from regular weight, style and stretch.
fn regularity(face: &fontdb::FaceInfo) -> u32 {
    let mut d = u32::from(face.weight.0.abs_diff(400));
    if face.style != fontdb::Style::Normal {
        d += 10_000;
    }
    if face.stretch != fontdb::Stretch::Normal {
        d += 10_000;
    }
    d
}

/// The most regular covering face of the first preferred family (in preference order) that
/// has one (ties go to database order); failing that, the most regular covering face of
/// any family.
fn pick<'a>(faces: &'a [Candidate], preferred: &[&str]) -> Option<&'a Candidate> {
    // `min_by_key` returns the first of equal minima, which is database order.
    preferred
        .iter()
        .find_map(|want| {
            faces
                .iter()
                .filter(|c| c.covers_cjk && c.families.iter().any(|f| f.eq_ignore_ascii_case(want)))
                .min_by_key(|c| c.regularity)
        })
        .or_else(|| {
            faces
                .iter()
                .filter(|c| c.covers_cjk)
                .min_by_key(|c| c.regularity)
        })
}

/// Find a CJK-capable system font, or `None` when this system has none.
///
/// This reads every system font directory once (on Linux, the ones fontconfig's
/// configuration names), which can take a second or more when the file cache is cold, and
/// it blocks the caller: call it once at startup. The chosen font file, possibly tens of
/// megabytes, is held in memory.
#[cfg(not(target_os = "ios"))]
pub fn system_cjk_font() -> Option<SystemFont> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    // fontdb knows no Android font directory. The system's fonts, Noto CJK among them, are
    // in the first; a device maker's own are in the second.
    #[cfg(target_os = "android")]
    for dir in ["/system/fonts", "/product/fonts"] {
        db.load_fonts_dir(dir);
    }
    choose(&db)
}

/// Find a CJK-capable system font, or `None` when this system has none.
///
/// iOS's sandbox refuses to list the system font directories, so the files come from
/// CoreText. The preferred families' files are read first, and every other font only when
/// none of them covers CJK. Blocks the caller: call it once at startup. The chosen font
/// file, possibly tens of megabytes, is held in memory.
#[cfg(target_os = "ios")]
pub fn system_cjk_font() -> Option<SystemFont> {
    let (preferred, rest) = split_preferred(&apple::font_files(), PREFERRED_FAMILIES);
    let mut db = fontdb::Database::new();
    for path in &preferred {
        load_file(&mut db, path);
    }
    if let Some(font) = choose(&db) {
        return Some(font);
    }
    for path in &rest {
        load_file(&mut db, path);
    }
    choose(&db)
}

#[cfg(target_os = "ios")]
fn load_file(db: &mut fontdb::Database, path: &std::path::Path) {
    if let Err(e) = db.load_font_file(path) {
        tracing::debug!(path = %path.display(), error = %e, "could not read a system font");
    }
}

/// The font files of `files` (family, path), split into those holding a `preferred` family
/// and the rest. Each file appears once, in its first position.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
fn split_preferred(
    files: &[(String, PathBuf)],
    preferred: &[&str],
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let is_preferred = |path: &PathBuf| {
        files.iter().any(|(family, p)| {
            p == path
                && preferred
                    .iter()
                    .any(|want| want.eq_ignore_ascii_case(family))
        })
    };
    let mut seen = std::collections::HashSet::new();
    files
        .iter()
        .map(|(_, path)| path)
        .filter(|path| seen.insert(*path))
        .cloned()
        .partition(is_preferred)
}

/// The CJK-capable face to use among `db`'s fonts.
fn choose(db: &fontdb::Database) -> Option<SystemFont> {
    let faces: Vec<&fontdb::FaceInfo> = db.faces().collect();

    let candidate = |(i, f): (usize, &&fontdb::FaceInfo)| Candidate {
        id_index: i,
        families: f.families.iter().map(|(n, _)| n.clone()).collect(),
        covers_cjk: db.with_face_data(f.id, covers_cjk).unwrap_or(false),
        regularity: regularity(f),
    };
    let is_preferred = |f: &fontdb::FaceInfo| {
        f.families.iter().any(|(name, _)| {
            PREFERRED_FAMILIES
                .iter()
                .any(|p| p.eq_ignore_ascii_case(name))
        })
    };
    // Coverage is checked for the preferred families' faces first, and for the whole
    // database only when none of them covers CJK.
    let preferred: Vec<Candidate> = faces
        .iter()
        .enumerate()
        .filter(|(_, f)| is_preferred(f))
        .map(candidate)
        .collect();
    let chosen_index = match pick(&preferred, PREFERRED_FAMILIES) {
        Some(c) => c.id_index,
        None => {
            let all: Vec<Candidate> = faces.iter().enumerate().map(candidate).collect();
            pick(&all, &[])?.id_index
        }
    };

    let chosen = faces[chosen_index];
    let family = chosen
        .families
        .first()
        .map(|(n, _)| n.clone())
        .unwrap_or_default();
    let weight = chosen.weight.0;
    db.with_face_data(chosen.id, |data, index| SystemFont {
        family,
        data: data.to_vec(),
        index,
        weight,
    })
}

/// Whether face `index` of `data` has every [`PROBE`] glyph.
fn covers_cjk(data: &[u8], index: u32) -> bool {
    ttf_parser::Face::parse(data, index)
        .map(|face| PROBE.iter().all(|c| face.glyph_index(*c).is_some()))
        .unwrap_or(false)
}

/// The system fonts as CoreText reports them.
#[cfg(target_os = "ios")]
mod apple {
    use std::path::PathBuf;

    use objc2_core_foundation::{CFArray, CFRetained, CFString, CFURL};
    use objc2_core_text::{
        CTFontCollection, CTFontDescriptor, kCTFontFamilyNameAttribute, kCTFontURLAttribute,
    };

    /// Every face available to the app, as (family, font file). A collection file appears
    /// once per face; faces with no file (made in memory) are left out.
    pub(super) fn font_files() -> Vec<(String, PathBuf)> {
        // SAFETY: no options are passed, and CoreText documents the result as an array of
        // font descriptors.
        let descriptors = unsafe {
            CTFontCollection::from_available_fonts(None)
                .matching_font_descriptors()
                .map(|a| CFRetained::cast_unchecked::<CFArray<CTFontDescriptor>>(a))
        };
        let Some(descriptors) = descriptors else {
            return Vec::new();
        };
        descriptors
            .iter()
            .filter_map(|d| {
                // SAFETY: the attribute keys are CoreText's own constants.
                let (family, url) = unsafe {
                    (
                        d.attribute(kCTFontFamilyNameAttribute)?,
                        d.attribute(kCTFontURLAttribute)?,
                    )
                };
                let family = family.downcast::<CFString>().ok()?.to_string();
                let path = url.downcast::<CFURL>().ok()?.to_file_path()?;
                Some((family, path))
            })
            .collect()
    }
}

/// Append a system CJK font to `fonts` as a fallback behind everything already in the
/// proportional and monospace families, keeping whatever else `fonts` holds. Use this to
/// merge into your own font definitions. Returns the family used, or `None` (logged) when
/// the system has none. See [`system_cjk_font`] for the cost of the lookup.
pub fn add_system_cjk_fallback(fonts: &mut egui::FontDefinitions) -> Option<String> {
    let Some(font) = system_cjk_font() else {
        tracing::warn!("no CJK-capable system font found; CJK text will not render");
        return None;
    };
    let mut data = egui::FontData::from_owned(font.data);
    data.index = font.index;
    fonts.font_data.insert(FONT_KEY.to_owned(), Arc::new(data));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(FONT_KEY.to_owned());
    }
    tracing::info!(family = %font.family, "using a system CJK font");
    Some(font.family)
}

/// Install a system CJK font as a fallback behind egui's default fonts. This starts from
/// egui's defaults and replaces the context's whole font set, so call it before (or
/// instead of) your own font setup; to merge into your own definitions use
/// [`add_system_cjk_fallback`]. Returns the family used, or `None` (logged) when the system
/// has none — CJK text then renders as boxes, nothing fails.
pub fn install_system_cjk_fallback(ctx: &egui::Context) -> Option<String> {
    let mut fonts = egui::FontDefinitions::default();
    let family = add_system_cjk_fallback(&mut fonts)?;
    ctx.set_fonts(fonts);
    Some(family)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cand(i: usize, fam: &str, covers: bool) -> Candidate {
        Candidate {
            id_index: i,
            families: vec![fam.to_owned()],
            covers_cjk: covers,
            regularity: 0,
        }
    }

    #[test]
    fn the_earliest_preferred_family_wins_regardless_of_database_order() {
        let faces = vec![cand(0, "Meiryo", true), cand(1, "Yu Gothic UI", true)];
        let got = pick(&faces, &["Yu Gothic UI", "Meiryo"]).unwrap();
        assert_eq!(got.id_index, 1);
    }

    #[test]
    fn family_names_match_case_insensitively() {
        let faces = vec![cand(0, "hiragino sans", true)];
        assert!(pick(&faces, &["Hiragino Sans"]).is_some());
    }

    #[test]
    fn a_preferred_family_without_cjk_glyphs_is_skipped() {
        let faces = vec![cand(0, "Yu Gothic UI", false), cand(1, "Meiryo", true)];
        assert_eq!(
            pick(&faces, &["Yu Gothic UI", "Meiryo"]).unwrap().id_index,
            1
        );
    }

    #[test]
    fn with_no_preferred_family_any_cjk_face_is_the_fallback() {
        let faces = vec![
            cand(0, "DejaVu Sans", false),
            cand(1, "Some CJK Font", true),
        ];
        assert_eq!(pick(&faces, &["Noto Sans CJK JP"]).unwrap().id_index, 1);
    }

    #[test]
    fn nothing_covering_cjk_is_none() {
        let faces = vec![cand(0, "DejaVu Sans", false)];
        assert!(pick(&faces, &["Noto Sans CJK JP"]).is_none());
    }

    /// Every supported desktop OS ships at least one of its preferred families; Windows CI
    /// always has Yu Gothic or Meiryo. macOS/Linux are checked by hand (Task 13).
    #[cfg(windows)]
    #[test]
    fn windows_finds_a_system_cjk_font() {
        let font = system_cjk_font().expect("Windows ships Yu Gothic / Meiryo");
        assert!(!font.data.is_empty());
        assert!(
            (300..=500).contains(&font.weight),
            "picked a non-regular face: {font:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn add_keeps_existing_fonts_and_appends_the_fallback_last() {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "mine".to_owned(),
            Arc::new(egui::FontData::from_static(&[0])),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "mine".to_owned());
        assert!(add_system_cjk_fallback(&mut fonts).is_some());
        assert!(fonts.font_data.contains_key("mine"));
        let prop = &fonts.families[&egui::FontFamily::Proportional];
        assert_eq!(prop.first().map(String::as_str), Some("mine"));
        assert_eq!(prop.last().map(String::as_str), Some(FONT_KEY));
    }

    fn file(family: &str, path: &str) -> (String, PathBuf) {
        (family.to_owned(), PathBuf::from(path))
    }

    #[test]
    fn the_preferred_families_files_come_apart_from_the_rest() {
        let files = vec![
            file("Helvetica", "/f/Helvetica.ttc"),
            file("Hiragino Sans", "/f/HiraginoW3.ttc"),
            file("PingFang SC", "/f/PingFang.ttc"),
            file("hiragino sans", "/f/HiraginoW6.ttc"),
        ];
        let (preferred, rest) = split_preferred(&files, &["Hiragino Sans"]);
        assert_eq!(
            preferred,
            [
                PathBuf::from("/f/HiraginoW3.ttc"),
                PathBuf::from("/f/HiraginoW6.ttc")
            ]
        );
        assert_eq!(
            rest,
            [
                PathBuf::from("/f/Helvetica.ttc"),
                PathBuf::from("/f/PingFang.ttc")
            ]
        );
    }

    /// A collection file lists one face per family it holds; it is loaded once, as preferred
    /// when any of its families is.
    #[test]
    fn a_file_with_several_families_is_listed_once() {
        let files = vec![
            file("PingFang SC", "/f/PingFang.ttc"),
            file("PingFang TC", "/f/PingFang.ttc"),
            file("Hiragino Sans", "/f/Mixed.ttc"),
            file("Other", "/f/Mixed.ttc"),
        ];
        let (preferred, rest) = split_preferred(&files, &["Hiragino Sans"]);
        assert_eq!(preferred, [PathBuf::from("/f/Mixed.ttc")]);
        assert_eq!(rest, [PathBuf::from("/f/PingFang.ttc")]);
    }

    /// Every phone ships one of its preferred families: Hiragino Sans on iOS, Noto Sans
    /// CJK on Android. Run on the simulator and the emulator by mobile-ci.yml.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    #[test]
    fn a_phone_finds_a_preferred_family() {
        let font = system_cjk_font().expect("no CJK-capable system font");
        assert!(
            PREFERRED_FAMILIES
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&font.family)),
            "picked a family outside the preferred list: {font:?}"
        );
    }

    #[test]
    fn the_regular_face_beats_a_bold_one_of_the_same_family() {
        let mut bold = cand(0, "Yu Gothic UI", true);
        bold.regularity = 300;
        let regular = cand(1, "Yu Gothic UI", true);
        let faces = vec![bold, regular];
        assert_eq!(pick(&faces, &["Yu Gothic UI"]).unwrap().id_index, 1);
    }

    #[test]
    fn a_preferred_family_with_no_cjk_face_falls_back_to_any_covering_face() {
        let faces = vec![cand(0, "Yu Gothic UI", false), cand(1, "Other", true)];
        assert_eq!(pick(&faces, &["Yu Gothic UI"]).unwrap().id_index, 1);
    }
}
