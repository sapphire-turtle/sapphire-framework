//! Text decided outside egui: a message id and its arguments, resolved where it is drawn.

use fluent_bundle::FluentValue;

/// A message to show: its id and arguments, for [`text`](Self::text) to look up in the
/// installed language. Plain functions tested without egui return this rather than text.
#[derive(Clone, PartialEq, Debug)]
pub struct Message {
    /// The message id, as in the `.ftl` files.
    pub id: &'static str,
    /// The arguments, by name.
    pub args: Vec<(&'static str, Arg)>,
}

impl Message {
    /// The message `id`, with no arguments yet.
    pub fn new(id: &'static str) -> Message {
        Message {
            id,
            args: Vec::new(),
        }
    }

    /// Add the argument `name`.
    pub fn arg(mut self, name: &'static str, value: impl Into<Arg>) -> Message {
        self.args.push((name, value.into()));
        self
    }

    /// The text, in the language installed in `ctx`. See [`tr_args`](super::tr_args).
    pub fn text(&self, ctx: &egui::Context) -> String {
        super::tr_args(ctx, self.id, &self.args)
    }
}

/// A message argument: text, or a number (which selects plural forms).
#[derive(Clone, PartialEq, Debug)]
pub enum Arg {
    /// Text, as is.
    Str(String),
    /// A number; Fluent picks plural forms by it.
    Num(f64),
}

impl Arg {
    pub(super) fn to_fluent(&self) -> FluentValue<'_> {
        match self {
            Arg::Str(s) => FluentValue::from(s.as_str()),
            Arg::Num(n) => FluentValue::from(*n),
        }
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Arg {
        Arg::Str(s)
    }
}

impl From<&str> for Arg {
    fn from(s: &str) -> Arg {
        Arg::Str(s.to_owned())
    }
}

impl From<f64> for Arg {
    fn from(n: f64) -> Arg {
        Arg::Num(n)
    }
}

/// Numbers past 2^53 lose precision, as any count shown to a person can afford.
macro_rules! from_number {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            #[allow(clippy::cast_precision_loss)]
            fn from(n: $t) -> Arg {
                Arg::Num(n as f64)
            }
        }
    )*};
}

from_number!(i32, i64, u32, u64, usize);
