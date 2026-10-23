//! One widget per setting kind.

use sapphire_bridge_api::settings::SecretState;
use sapphire_bridge_api::{EntryValue, Kind, SettingDef, Value};

use super::model::{Edit, parse_text};

/// Render `def`'s widget, showing the drafted text when there is a draft and `shown`
/// otherwise. Returns the new edit when the user changed it; never while `enabled` is false.
/// A secret's widget is empty until typed into, whatever `shown` holds.
pub(crate) fn value_widget(
    ui: &mut egui::Ui,
    def: &SettingDef,
    shown: Option<&EntryValue>,
    draft: Option<&Edit>,
    enabled: bool,
) -> Option<Edit> {
    // A drafted unset shows what the key falls back to: the default.
    let default = def.default.clone().map(EntryValue::Value);
    let shown = match draft {
        Some(Edit::Unset) => default.as_ref(),
        _ => shown,
    };
    let current = current_value(draft, shown).or(def.default.as_ref());
    let mut text = match draft {
        Some(Edit::Set { text, .. }) => text.clone(),
        _ => shown_text(&def.kind, shown),
    };
    let changed = ui
        .add_enabled_ui(enabled, |ui| match &def.kind {
            Kind::Bool => {
                let mut on = matches!(current, Some(Value::Bool(true)));
                let changed = ui.checkbox(&mut on, "").changed();
                text = on.to_string();
                changed
            }
            Kind::Int { min, max } => {
                let mut n = match current {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                }
                .clamp(*min, *max);
                let changed = ui
                    .add(egui::DragValue::new(&mut n).range(*min..=*max))
                    .changed();
                text = n.to_string();
                changed
            }
            Kind::Enum { choices } => {
                let before = text.clone();
                egui::ComboBox::from_id_salt(&def.key)
                    .selected_text(text.clone())
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            ui.selectable_value(&mut text, choice.clone(), choice);
                        }
                    });
                text != before
            }
            Kind::List { .. } => ui
                .add(
                    egui::TextEdit::multiline(&mut text)
                        .id_salt(&def.key)
                        .desired_rows(3)
                        .hint_text("one per line"),
                )
                .changed(),
            Kind::Secret => {
                let hint = match shown {
                    Some(EntryValue::Secret(SecretState::Set)) => "set",
                    _ => "not set",
                };
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id_salt(&def.key)
                        .password(true)
                        .hint_text(hint),
                )
                .changed()
            }
            Kind::String { .. } | Kind::Url | Kind::Path | Kind::SocketAddr => ui
                .add(egui::TextEdit::singleline(&mut text).id_salt(&def.key))
                .changed(),
        })
        .inner;
    (changed && enabled).then(|| Edit::Set {
        parsed: parse_text(&def.kind, &text),
        text,
    })
}

/// The value the widget stands at: the draft's when it parses, else the shown one.
fn current_value<'a>(draft: Option<&'a Edit>, shown: Option<&'a EntryValue>) -> Option<&'a Value> {
    match (draft, shown) {
        (Some(Edit::Set { parsed: Ok(v), .. }), _) => Some(v),
        (_, Some(EntryValue::Value(v))) => Some(v),
        _ => None,
    }
}

/// The text a field shows for a value with no draft: a list one item per line, a secret
/// nothing at all.
pub(crate) fn shown_text(kind: &Kind, shown: Option<&EntryValue>) -> String {
    match (kind, shown) {
        (Kind::Secret, _) => String::new(),
        (_, Some(EntryValue::Value(Value::List(items)))) => items
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        (_, Some(EntryValue::Value(v))) => v.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use sapphire_bridge_api::Scope;
    use sapphire_bridge_api::settings::{Apply, Process};

    use super::*;

    /// A device-scoped definition of `kind`.
    pub(crate) fn def(key: &str, kind: Kind) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind,
            scopes: vec![Scope::Device],
            overridable: false,
            apply: Apply::Live,
            default: None,
            section: "S".into(),
            doc: format!("what {key} does"),
            process: Process::App,
        }
    }

    /// One key of every kind.
    pub(crate) fn every_kind() -> Vec<SettingDef> {
        vec![
            def("k.bool", Kind::Bool),
            def("k.int", Kind::Int { min: 1, max: 9 }),
            def(
                "k.enum",
                Kind::Enum {
                    choices: vec!["a".into(), "b".into()],
                },
            ),
            def("k.string", Kind::String { non_empty: true }),
            def("k.url", Kind::Url),
            def("k.path", Kind::Path),
            def("k.addr", Kind::SocketAddr),
            def(
                "k.list",
                Kind::List {
                    item: Box::new(Kind::Int { min: 0, max: 99 }),
                },
            ),
            def("k.secret", Kind::Secret),
        ]
    }

    /// Every text drawn over two frames of `ui`.
    pub(crate) fn drawn_text(mut ui: impl FnMut(&mut egui::Ui)) -> String {
        fn collect(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push_str(t.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut out = String::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), &mut ui);
            output.textures_delta.clear();
            out.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut out);
            }
        }
        out
    }

    #[test]
    fn every_kind_has_a_widget_and_emits_nothing_untouched() {
        let shown = [
            Some(EntryValue::Value(Value::Bool(true))),
            Some(EntryValue::Value(Value::Int(4))),
            Some(EntryValue::Value(Value::String("b".into()))),
            None,
            Some(EntryValue::Value(Value::String("https://x".into()))),
            Some(EntryValue::Unset),
            Some(EntryValue::Value(Value::String("127.0.0.1:1".into()))),
            Some(EntryValue::Value(Value::List(vec![Value::Int(1)]))),
            Some(EntryValue::Secret(SecretState::Set)),
        ];
        crate::test_support::render([600.0, 800.0], |ui| {
            for (def, shown) in every_kind().iter().zip(&shown) {
                for enabled in [true, false] {
                    ui.push_id((&def.key, enabled), |ui| {
                        assert!(value_widget(ui, def, shown.as_ref(), None, enabled).is_none());
                    });
                }
            }
        });
    }

    #[test]
    fn a_secret_field_never_shows_the_value() {
        let secret = def("k.secret", Kind::Secret);
        // Even a value the server should never send is not shown.
        let leaked = EntryValue::Value(Value::String("server-value".into()));
        assert_eq!(shown_text(&Kind::Secret, Some(&leaked)), "");
        let typed = Edit::Set {
            text: "typed-secret".into(),
            parsed: Ok(Value::String("typed-secret".into())),
        };
        let drawn = drawn_text(|ui| {
            value_widget(ui, &secret, Some(&leaked), Some(&typed), true);
        });
        assert!(!drawn.contains("server-value") && !drawn.contains("typed-secret"));
        let drawn = drawn_text(|ui| {
            value_widget(
                ui,
                &secret,
                Some(&EntryValue::Secret(SecretState::Set)),
                None,
                true,
            );
        });
        assert!(drawn.contains("set"), "the hint says it is set: {drawn}");
    }

    #[test]
    fn shown_text_lists_one_item_per_line() {
        let list = EntryValue::Value(Value::List(vec![Value::Int(1), Value::Int(2)]));
        assert_eq!(shown_text(&Kind::Bool, Some(&list)), "1\n2");
        assert_eq!(shown_text(&Kind::Bool, Some(&EntryValue::Unset)), "");
    }
}
