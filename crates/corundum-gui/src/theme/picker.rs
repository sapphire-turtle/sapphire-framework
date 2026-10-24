//! [`TintPicker`]: pick a [`Tint`] — from the theme's swatches, at random, or anywhere on a
//! disk of hues (around) and chromas (outwards).

use super::{Swatch, Theme, Tint, Tone};

use egui::{Color32, Pos2, Rect, Sense, Stroke, vec2};

use crate::tr;

/// The chips' side, and the gap between them.
const CHIP: f32 = 24.0;
const GAP: f32 = 4.0;
/// The disk's widest diameter, and its mesh.
const DISK: f32 = 160.0;
const SECTORS: usize = 48;
const RINGS: usize = 8;
/// The swatches on the palette, between Default and Random.
const PALETTE: [Swatch; 8] = [
    Swatch::Neutral,
    Swatch::Red,
    Swatch::Orange,
    Swatch::Yellow,
    Swatch::Green,
    Swatch::Cyan,
    Swatch::Blue,
    Swatch::Purple,
];

/// Picks a [`Tint`]: a palette (Default, the theme's swatches, Random) over a disk of hues
/// around and chromas outwards, and optionally a preview of the tint in use.
///
/// It edits an `Option<Tint>`: `None` is unset, shown as the `default` — a label's
/// [`Tint::from_bytes`] of its name, say — and the Default chip sets it back.
///
/// ```no_run
/// # use corundum_gui::theme::{Tint, TintPicker, Tone};
/// # fn example(ui: &mut egui::Ui, colour: &mut Option<Tint>) {
/// let default = Tint::from_bytes(b"bug");
/// if TintPicker::new(colour, default).tone(Tone::Raised).show(ui) {
///     // save `colour`
/// }
/// # }
/// ```
pub struct TintPicker<'a> {
    value: &'a mut Option<Tint>,
    default: Tint,
    tone: Tone,
    preview: bool,
}

impl<'a> TintPicker<'a> {
    /// A picker over `value`, showing `default` while it is `None`.
    pub fn new(value: &'a mut Option<Tint>, default: Tint) -> TintPicker<'a> {
        TintPicker {
            value,
            default,
            tone: Tone::Text,
            preview: false,
        }
    }

    /// Draw the chips and the disk at `tone`: where the tint will be used. Text by default.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Show the tint as link text, a label and a selected row under the disk.
    pub fn preview(mut self, on: bool) -> Self {
        self.preview = on;
        self
    }

    /// Draw the picker. `true` once a change is made — a click, or the end of a drag (the
    /// value follows the drag meanwhile) — so save the value then.
    pub fn show(mut self, ui: &mut egui::Ui) -> bool {
        let theme = super::current(ui.ctx());
        let before = *self.value;
        let mut dragging = false;
        let mut rects = PickerRects {
            chips: Vec::new(),
            die: Rect::NOTHING,
            disk: Rect::NOTHING,
            preview: Rect::NOTHING,
        };
        let id = ui.next_auto_id();
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            ui.horizontal_wrapped(|ui| {
                self.palette_ui(ui, &theme, &mut rects);
            });
            let (disk, held) = self.disk_ui(ui, &theme);
            rects.disk = disk;
            dragging = held;
            if self.preview {
                rects.preview = preview_ui(ui, &theme, self.value.unwrap_or(self.default));
            }
        });
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(PickerRects::id(), rects.clone()));
        let changed = *self.value != before;
        let held_id = id.with("held");
        let mut held = ui.data(|d| d.get_temp(held_id)).unwrap_or(false);
        let made = report(changed, dragging, &mut held);
        ui.data_mut(|d| d.insert_temp(held_id, held));
        made
    }

    fn palette_ui(&mut self, ui: &mut egui::Ui, theme: &Theme, rects: &mut PickerRects) {
        let chips = &mut rects.chips;
        let shown = self.value.unwrap_or(self.default);
        let fill = theme.tint(self.tone, self.default);
        let r = chip(
            ui,
            theme,
            fill,
            self.value.is_none(),
            &tr!(ui, "corundum-theme-default"),
        );
        paint_icon(ui, r.rect, crate::icons::rotate_ccw(), on(theme, self.tone));
        if r.clicked() {
            *self.value = None;
        }
        chips.push(r.rect);
        for s in PALETTE {
            let selected = self.value.is_some() && shown == Tint::Swatch(s);
            let r = chip(
                ui,
                theme,
                theme.color(self.tone, s),
                selected,
                &tr!(ui, swatch_id(s)),
            );
            if r.clicked() {
                *self.value = Some(Tint::Swatch(s));
            }
            chips.push(r.rect);
        }
        let r = chip(
            ui,
            theme,
            Color32::TRANSPARENT,
            false,
            &tr!(ui, "corundum-theme-random"),
        );
        let painter = ui.painter();
        let (c, radius) = (r.rect.center(), CHIP * 0.4);
        for k in 0..24 {
            let at = |k: f32| {
                let a = (k * 15.0).to_radians();
                c + radius * vec2(a.cos(), -a.sin())
            };
            let colour = theme.tint(
                self.tone,
                Tint::Custom {
                    chroma: Tint::MAX_CHROMA,
                    hue: k as f32 * 15.0,
                },
            );
            painter.line_segment([at(k as f32), at(k as f32 + 1.0)], Stroke::new(3.0, colour));
        }
        // Inside the ring, which runs at 0.4 of the chip from its centre.
        rects.die = paint_icon(
            ui,
            r.rect.shrink(2.0),
            crate::icons::dice_5(),
            theme.color(Tone::Text, Swatch::Neutral),
        );
        if r.clicked() {
            *self.value = Some(Tint::random());
        }
        rects.chips.push(r.rect);
    }

    /// The disk; whether the pointer is held on it (from the press to the release, so a
    /// drag is reported once, when it ends).
    fn disk_ui(&mut self, ui: &mut egui::Ui, theme: &Theme) -> (Rect, bool) {
        let side = DISK.min(ui.available_width()).max(CHIP);
        let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
        let (centre, radius) = (rect.center(), side / 2.0);
        let at = |r: f32, hue: f32| {
            let a = hue.to_radians();
            centre + r * radius * vec2(a.cos(), -a.sin())
        };
        let mut mesh = egui::Mesh::default();
        for ring in 0..=RINGS {
            let r = ring as f32 / RINGS as f32;
            for sector in 0..=SECTORS {
                let hue = sector as f32 * 360.0 / SECTORS as f32;
                let tint = Tint::Custom {
                    chroma: r * Tint::MAX_CHROMA,
                    hue,
                };
                mesh.colored_vertex(at(r, hue), theme.tint(self.tone, tint));
            }
        }
        let row = (SECTORS + 1) as u32;
        for ring in 0..RINGS as u32 {
            for sector in 0..SECTORS as u32 {
                let (a, b) = (ring * row + sector, ring * row + sector + 1);
                let (c, d) = (a + row, b + row);
                mesh.add_triangle(a, b, d);
                mesh.add_triangle(a, d, c);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));

        if (response.is_pointer_button_down_on() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            *self.value = Some(from_pointer(pos, centre, radius));
        }
        let (chroma, hue) = theme.chroma_hue(self.value.unwrap_or(self.default));
        let mark = at(chroma / Tint::MAX_CHROMA, hue);
        ui.painter()
            .circle_stroke(mark, 5.0, Stroke::new(3.0, Color32::BLACK));
        ui.painter()
            .circle_stroke(mark, 5.0, Stroke::new(1.5, Color32::WHITE));
        (rect, response.is_pointer_button_down_on())
    }
}

/// The message naming a palette swatch.
fn swatch_id(swatch: Swatch) -> &'static str {
    match swatch {
        Swatch::Neutral => "corundum-theme-swatch-neutral",
        Swatch::Red => "corundum-theme-swatch-red",
        Swatch::Orange => "corundum-theme-swatch-orange",
        Swatch::Yellow => "corundum-theme-swatch-yellow",
        Swatch::Green => "corundum-theme-swatch-green",
        Swatch::Cyan => "corundum-theme-swatch-cyan",
        Swatch::Blue => "corundum-theme-swatch-blue",
        Swatch::Purple => "corundum-theme-swatch-purple",
        // The accent swatch is the Default chip.
        Swatch::Accent => "corundum-theme-default",
    }
}

/// A chip of `fill`, ringed when `selected`, named `name` on hover.
fn chip(
    ui: &mut egui::Ui,
    theme: &Theme,
    fill: Color32,
    selected: bool,
    name: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(CHIP, CHIP), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, fill);
    if selected {
        let ring = Stroke::new(2.0, theme.color(Tone::Text, Swatch::Neutral));
        painter.rect_stroke(rect.expand(2.0), 6.0, ring, egui::StrokeKind::Outside);
    }
    response.on_hover_text(name)
}

/// `icon` in `colour`, 4 points inside `rect`; where it went.
fn paint_icon(
    ui: &egui::Ui,
    rect: Rect,
    icon: egui::ImageSource<'static>,
    colour: Color32,
) -> Rect {
    let rect = rect.shrink(4.0);
    egui::Image::new(icon).tint(colour).paint_at(ui, rect);
    rect
}

/// A colour that stands out on a fill at `tone`: dark on the text side's light fills in a
/// dark theme, and so on.
fn on(theme: &Theme, tone: Tone) -> Color32 {
    let text_side = matches!(tone, Tone::Muted | Tone::Text | Tone::Strong);
    let opposite = if text_side { Tone::Base } else { Tone::Text };
    theme.color(opposite, Swatch::Neutral)
}

/// The tint in use, small and not clickable or selectable: link text, a label, a selected
/// row. Where it went.
fn preview_ui(ui: &mut egui::Ui, theme: &Theme, tint: Tint) -> Rect {
    use egui::RichText;
    let text = |ui: &mut egui::Ui, words: RichText| {
        ui.add(egui::Label::new(words).selectable(false));
    };
    ui.horizontal(|ui| {
        let (link, label_text, selected) = (
            tr!(ui, "corundum-theme-link"),
            tr!(ui, "corundum-theme-label"),
            tr!(ui, "corundum-theme-selected"),
        );
        text(
            ui,
            RichText::new(link)
                .small()
                .underline()
                .color(theme.tint(Tone::Text, tint)),
        );
        let label = |ui: &mut egui::Ui, fill: Tone, colour: Color32, words: &str| {
            egui::Frame::new()
                .fill(theme.tint(fill, tint))
                .corner_radius(6.0)
                .inner_margin(vec2(6.0, 1.0))
                .show(ui, |ui| {
                    text(ui, RichText::new(words).small().color(colour))
                });
        };
        label(ui, Tone::Raised, theme.tint(Tone::Text, tint), &label_text);
        label(
            ui,
            Tone::Pressed,
            theme.color(Tone::Text, Swatch::Neutral),
            &selected,
        );
    })
    .response
    .rect
}

/// The tint at `pos` on a disk at `centre` of `radius`: hue 0° at the right, counter-
/// clockwise; outside the rim reads as on it.
fn from_pointer(pos: Pos2, centre: Pos2, radius: f32) -> Tint {
    let d = pos - centre;
    Tint::Custom {
        chroma: (d.length() / radius).min(1.0) * Tint::MAX_CHROMA,
        hue: (-d.y).atan2(d.x).to_degrees().rem_euclid(360.0),
    }
}

/// Whether a change is made this frame. A change during a drag is `held` until the drag
/// ends, so an app saving on each change saves once per drag, not once per frame.
pub(super) fn report(changed: bool, dragging: bool, held: &mut bool) -> bool {
    if dragging {
        *held |= changed;
        false
    } else {
        changed | std::mem::take(held)
    }
}

/// Where the last picker drew its chips and disk, for the tests to click.
#[derive(Clone)]
pub(super) struct PickerRects {
    pub(super) chips: Vec<Rect>,
    pub(super) die: Rect,
    pub(super) disk: Rect,
    pub(super) preview: Rect,
}

#[cfg(test)]
impl PickerRects {
    pub(super) fn id() -> egui::Id {
        egui::Id::new("corundum_tint_picker_rects")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Contrast, ThemeSettings, install};

    /// The frames of a test: a context with the theme and the loaders, and a picker
    /// redrawn each frame over `value`.
    struct Rig {
        ctx: egui::Context,
        value: Option<Tint>,
        made: Vec<bool>,
        cursor: egui::CursorIcon,
    }

    impl Rig {
        fn new(value: Option<Tint>) -> Rig {
            let ctx = egui::Context::default();
            install(&ctx, &ThemeSettings::default());
            crate::icons::install_loaders(&ctx);
            Rig {
                ctx,
                value,
                made: Vec::new(),
                cursor: egui::CursorIcon::Default,
            }
        }

        fn frame(&mut self, width: f32, preview: bool, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            };
            let (value, made) = (&mut self.value, &mut self.made);
            let mut output = self.ctx.run_ui(input, |ui| {
                made.push(
                    TintPicker::new(value, Tint::Swatch(Swatch::Blue))
                        .preview(preview)
                        .show(ui),
                );
            });
            output.textures_delta.clear();
            self.cursor = output.platform_output.cursor_icon;
        }

        fn rects(&self) -> PickerRects {
            self.ctx
                .data(|d| d.get_temp::<PickerRects>(PickerRects::id()))
                .expect("the picker records its rects in tests")
        }

        /// Lay out, then click at `at(rects)`; what `show` returned meanwhile.
        fn click(&mut self, at: impl Fn(&PickerRects) -> egui::Pos2) -> Vec<bool> {
            self.frame(400.0, false, vec![]);
            self.frame(400.0, false, vec![]);
            let pos = at(&self.rects());
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.made.clear();
            self.frame(400.0, false, vec![egui::Event::PointerMoved(pos)]);
            self.frame(400.0, false, vec![button(true)]);
            self.frame(400.0, false, vec![button(false)]);
            self.frame(400.0, false, vec![]);
            std::mem::take(&mut self.made)
        }
    }

    fn once(made: &[bool]) -> bool {
        made.iter().filter(|m| **m).count() == 1
    }

    #[test]
    fn renders_with_and_without_preview() {
        for (width, preview) in [(400.0, false), (400.0, true), (120.0, true)] {
            let mut rig = Rig::new(Some(Tint::Swatch(Swatch::Red)));
            rig.frame(width, preview, vec![]);
            rig.frame(width, preview, vec![]);
            let rects = rig.rects();
            assert_eq!(rects.chips.len(), 10);
            assert!(rects.disk.is_positive());
            for r in rects.chips.iter().chain([&rects.disk]) {
                assert!(r.right() <= width + 0.5, "{r:?} overflows {width}");
            }
            assert!(rig.made.iter().all(|m| !m));
        }
    }

    #[test]
    fn the_preview_follows_the_language() {
        fn collect(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => out.push_str(t.galley.text()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::i18n::install(
            &ctx,
            &crate::i18n::LanguageSettings {
                language: crate::i18n::Language::Explicit("ja".into()),
            },
        );
        let theme = Theme::new(false, Contrast::Medium, Tint::Swatch(Swatch::Blue));
        let mut drawn = String::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                preview_ui(ui, &theme, Tint::Swatch(Swatch::Blue));
            });
            output.textures_delta.clear();
            drawn.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut drawn);
            }
        }
        for ja in ["リンク", "ラベル", "選択中"] {
            assert!(drawn.contains(ja), "{ja} missing from {drawn}");
        }
    }

    #[test]
    fn default_chip_unsets() {
        let mut rig = Rig::new(Some(Tint::Swatch(Swatch::Red)));
        let made = rig.click(|r| r.chips[0].center());
        assert_eq!(rig.value, None);
        assert!(once(&made), "{made:?}");
    }

    #[test]
    fn colour_chip_sets_swatch() {
        let mut rig = Rig::new(None);
        let made = rig.click(|r| r.chips[3].center());
        assert_eq!(rig.value, Some(Tint::Swatch(Swatch::Orange)));
        assert!(once(&made), "{made:?}");
    }

    #[test]
    fn disk_centre_is_grey() {
        let mut rig = Rig::new(None);
        let made = rig.click(|r| r.disk.center());
        match rig.value {
            Some(Tint::Custom { chroma, .. }) => assert!(chroma < 0.01, "{chroma}"),
            other => panic!("{other:?}"),
        }
        assert!(once(&made), "{made:?}");
    }

    #[test]
    fn random_chip_sets_custom() {
        let mut rig = Rig::new(None);
        let made = rig.click(|r| r.chips[9].center());
        assert!(
            matches!(rig.value, Some(Tint::Custom { .. })),
            "{:?}",
            rig.value
        );
        assert!(once(&made), "{made:?}");
    }

    #[test]
    fn a_drag_on_the_disk_is_reported_once() {
        let mut rig = Rig::new(None);
        rig.frame(400.0, false, vec![]);
        rig.frame(400.0, false, vec![]);
        let start = rig.rects().disk.center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        rig.made.clear();
        rig.frame(400.0, false, vec![egui::Event::PointerMoved(start)]);
        rig.frame(400.0, false, vec![button(start, true)]);
        let mut pos = start;
        for _ in 0..4 {
            pos += egui::vec2(10.0, 0.0);
            rig.frame(400.0, false, vec![egui::Event::PointerMoved(pos)]);
        }
        rig.frame(400.0, false, vec![button(pos, false)]);
        rig.frame(400.0, false, vec![]);
        assert!(once(&rig.made), "{:?}", rig.made);
        match rig.value {
            Some(Tint::Custom { chroma, hue }) => {
                assert!(chroma > 0.05, "{chroma}");
                assert!(hue < 1.0 || hue > 359.0, "dragged right: {hue}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_die_is_legible() {
        let mut rig = Rig::new(None);
        rig.frame(400.0, false, vec![]);
        rig.frame(400.0, false, vec![]);
        let die = rig.rects().die;
        assert!(die.width() >= 12.0, "{die:?}");
    }

    #[test]
    fn preview_text_is_not_selectable() {
        let mut rig = Rig::new(Some(Tint::Swatch(Swatch::Red)));
        rig.frame(400.0, true, vec![]);
        rig.frame(400.0, true, vec![]);
        let preview = rig.rects().preview;
        let over = preview.left_center() + egui::vec2(6.0, 0.0);
        rig.frame(400.0, true, vec![egui::Event::PointerMoved(over)]);
        rig.frame(400.0, true, vec![]);
        assert_ne!(rig.cursor, egui::CursorIcon::Text, "over {preview:?}");
    }

    #[test]
    fn pointer_outside_clamps_to_rim() {
        let centre = egui::pos2(100.0, 100.0);
        let Tint::Custom { chroma, hue } = from_pointer(egui::pos2(100.0, 0.0), centre, 50.0)
        else {
            panic!()
        };
        assert_eq!(chroma, Tint::MAX_CHROMA);
        assert!((hue - 90.0).abs() < 1e-3, "up is 90°: {hue}");
        let Tint::Custom { hue, .. } = from_pointer(egui::pos2(150.0, 100.0), centre, 50.0) else {
            panic!()
        };
        assert!(hue.abs() < 1e-3, "right is 0°: {hue}");
    }

    #[test]
    fn accent_swatch_marks_the_accent() {
        let accent = Tint::Custom {
            chroma: 0.2,
            hue: 40.0,
        };
        let theme = Theme::new(true, Contrast::Medium, accent);
        assert_eq!(theme.chroma_hue(Tint::Swatch(Swatch::Accent)), (0.2, 40.0));
        assert_eq!(
            theme.chroma_hue(Tint::Swatch(Swatch::Red)),
            Swatch::Red.chroma_hue()
        );
    }

    #[test]
    fn a_drag_is_reported_once_it_ends() {
        let mut held = false;
        assert!(!report(true, true, &mut held), "mid-drag");
        assert!(!report(true, true, &mut held), "mid-drag");
        assert!(report(false, false, &mut held), "the drag ended");
        assert!(!report(false, false, &mut held), "only once");
        assert!(report(true, false, &mut held), "a click");
        assert!(
            !report(false, true, &mut held),
            "a drag that changed nothing"
        );
        assert!(!report(false, false, &mut held));
    }
}
