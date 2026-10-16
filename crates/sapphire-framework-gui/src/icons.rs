//! The Lucide icons (ISC license, `assets/icons/LICENSE-Lucide`) the components draw.
//!
//! Embedded with [`egui::include_image!`] and decoded by `egui_extras`'s SVG loader,
//! which [`install_loaders`] puts in place. The SVGs stroke in white, so
//! [`egui::Image::tint`] colours them — the text colour, as the apps' own icons are (the
//! apps use Lucide too: sapphire-journal's toolbar does).

macro_rules! icon {
    ($name:literal) => {
        egui::include_image!(concat!("../assets/icons/", $name, ".svg"))
    };
}

/// The rail's foot, which opens Settings.
pub fn settings() -> egui::ImageSource<'static> {
    icon!("settings")
}

/// Back, from a Settings page to the list (compact).
pub fn arrow_left() -> egui::ImageSource<'static> {
    icon!("arrow-left")
}

/// Put the image loaders in place, SVG included, unless they already are. The shell calls
/// it; an app drawing these icons without one calls it once at startup.
pub fn install_loaders(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
}

/// `src` as a `size`-point square in the text colour.
pub(crate) fn image(
    ui: &egui::Ui,
    src: egui::ImageSource<'static>,
    size: f32,
) -> egui::Image<'static> {
    egui::Image::new(src)
        .fit_to_exact_size(egui::vec2(size, size))
        .tint(ui.visuals().text_color())
}
