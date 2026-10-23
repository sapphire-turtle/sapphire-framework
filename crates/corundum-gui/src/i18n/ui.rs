//! [`settings_ui`]: the language picker, for an app's settings screen.

use super::{Language, LanguageSettings, available, choose, install, name_of, system_languages};
use crate::tr;

/// A drop-down of the languages: the system's first, with the language it resolves to
/// now, then each available language by its own name. A choice is [`install`]ed at once and
/// `true` comes that frame, so save `settings` then.
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut LanguageSettings) -> bool {
    let langs = available(ui.ctx());
    let selected = match &settings.language {
        Language::System => system_entry(ui, &langs),
        Language::Explicit(tag) => langs
            .iter()
            .find(|l| l.to_string() == *tag)
            .map_or_else(|| tag.clone(), |l| name_of(ui.ctx(), l)),
    };
    let before = settings.clone();
    egui::ComboBox::from_id_salt("corundum_language")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            let system = system_entry(ui, &langs);
            ui.selectable_value(&mut settings.language, Language::System, system);
            for lang in &langs {
                let name = name_of(ui.ctx(), lang);
                ui.selectable_value(
                    &mut settings.language,
                    Language::Explicit(lang.to_string()),
                    name,
                );
            }
        });
    let changed = *settings != before;
    if changed {
        apply(ui.ctx(), settings);
    }
    changed
}

/// The System entry: "System (日本語)", naming the language the system's resolves to.
fn system_entry(ui: &egui::Ui, langs: &[super::LanguageIdentifier]) -> String {
    let resolved = choose(&system_languages(), langs);
    tr!(
        ui,
        "corundum-language-system",
        language = name_of(ui.ctx(), &resolved)
    )
}

/// What a change does: install the new settings.
fn apply(ctx: &egui::Context, settings: &LanguageSettings) {
    install(ctx, settings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::current;
    use crate::test_support::{render, render_in};

    #[test]
    fn settings_ui_renders_without_a_change() {
        let mut settings = LanguageSettings::default();
        let mut changed = false;
        render([600.0, 400.0], |ui| {
            changed |= settings_ui(ui, &mut settings)
        });
        assert!(!changed);
        assert_eq!(settings, LanguageSettings::default());
    }

    #[test]
    fn apply_installs() {
        let ctx = egui::Context::default();
        let settings = LanguageSettings {
            language: Language::Explicit("ja".into()),
        };
        render_in(&ctx, [600.0, 400.0], |ui| apply(ui.ctx(), &settings));
        assert_eq!(current(&ctx).to_string(), "ja");
    }
}
