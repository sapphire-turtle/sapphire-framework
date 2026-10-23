//! Helpers for rendering the components in unit tests.

/// Run `ui` over two frames of a `size` screen (the first lays out, the second draws).
pub(crate) fn render(size: [f32; 2], ui: impl FnMut(&mut egui::Ui)) {
    render_in(&egui::Context::default(), size, ui);
}

/// As [`render`], in `ctx`.
pub(crate) fn render_in(ctx: &egui::Context, size: [f32; 2], mut ui: impl FnMut(&mut egui::Ui)) {
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, &mut ui);
        // Unapplied texture deltas panic on drop; there is no renderer here.
        output.textures_delta.clear();
    }
}
