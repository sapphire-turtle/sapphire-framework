//! Errors of the settings registry.

use std::path::PathBuf;

use sapphire_bridge_api::ConfigFieldError;
use sapphire_bridge_api::settings::Scope;

use crate::owner::FieldError;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Why a settings operation failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No setting has this key.
    #[error("unknown setting `{0}`")]
    UnknownKey(String),
    /// The key exists but may not be set in this scope.
    #[error(
        "`{key}` cannot be set in the {scope} scope (allowed: {})",
        scope_list(allowed)
    )]
    ScopeNotAllowed {
        /// The key.
        key: String,
        /// The scope that was asked for.
        scope: Scope,
        /// The scopes the key accepts.
        allowed: Vec<Scope>,
    },
    /// The value does not fit the key's kind.
    #[error("invalid value for `{key}`: {message}")]
    InvalidValue {
        /// The key.
        key: String,
        /// What is wrong with the value.
        message: String,
    },
    /// A secret was given as text on the command line.
    #[error("`{0}` is a secret; give it with --stdin or a prompt, not on the command line")]
    SecretOnCommandLine(String),
    /// Two owners define the same key.
    #[error("setting `{0}` is defined by more than one owner")]
    Duplicate(String),
    /// A key uses a top-level name reserved for another owner.
    #[error("setting `{key}` uses the name `{name}`, which is reserved for another owner")]
    Reserved {
        /// The offending key.
        key: String,
        /// The reserved top-level name.
        name: String,
    },
    /// A definition breaks the registry's rules: it names no scope, or it is a secret
    /// outside the device scope.
    #[error("setting `{key}` is defined wrongly: {message}")]
    InvalidDef {
        /// The offending key.
        key: String,
        /// Which rule it breaks.
        message: String,
    },
    /// A settings file is not valid TOML, or holds a value of an unsupported type.
    #[error("cannot read {}: {message}", file.display())]
    Unparsable {
        /// The file.
        file: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// An owner refused the edited settings.
    #[error("settings rejected: {}", rejected(.0))]
    Rejected(Vec<FieldError>),
    /// A filesystem error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// The keys this error names, for a client to show under each field; empty when it
    /// names none. The messages never hold a value.
    pub fn field_errors(&self) -> Vec<ConfigFieldError> {
        let one = |key: &String, message: String| {
            vec![ConfigFieldError {
                key: key.clone(),
                message,
            }]
        };
        match self {
            Self::UnknownKey(key) => one(key, "unknown setting".to_owned()),
            Self::ScopeNotAllowed {
                key,
                scope,
                allowed,
            } => one(
                key,
                format!(
                    "cannot be set in the {scope} scope (allowed: {})",
                    scope_list(allowed)
                ),
            ),
            Self::InvalidValue { key, message } => one(key, message.clone()),
            Self::SecretOnCommandLine(key) => one(
                key,
                "is a secret; give it with --stdin or a prompt, not on the command line".to_owned(),
            ),
            Self::Rejected(errors) => errors
                .iter()
                .filter_map(|e| {
                    Some(ConfigFieldError {
                        key: e.key.clone()?,
                        message: e.message.clone(),
                    })
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn scope_list(scopes: &[Scope]) -> String {
    scopes
        .iter()
        .map(Scope::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn rejected(errors: &[FieldError]) -> String {
    errors
        .iter()
        .map(FieldError::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// A TOML parse error of `text` as the line and column it is at and what is wrong, never
/// the text itself: a settings file may hold an application's tokens beside the settings.
/// `span` and `message` are the error's own (`toml::de::Error`, `toml_edit::TomlError`).
pub fn describe_toml_error(
    text: &str,
    span: Option<std::ops::Range<usize>>,
    message: &str,
) -> String {
    let message = message.trim();
    let Some(span) = span else {
        return message.to_owned();
    };
    let before = &text[..span.start.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("line {line}, column {column}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_errors_name_the_keys() {
        let key = || "net.discovery".to_owned();
        let keyed = |e: Error| {
            e.field_errors()
                .into_iter()
                .map(|f| f.key)
                .collect::<Vec<_>>()
        };
        let expect = vec![key()];
        assert_eq!(keyed(Error::UnknownKey(key())), expect);
        assert_eq!(
            keyed(Error::ScopeNotAllowed {
                key: key(),
                scope: Scope::Workspace,
                allowed: vec![Scope::Device],
            }),
            expect
        );
        assert_eq!(
            keyed(Error::InvalidValue {
                key: key(),
                message: "expected a bool".into()
            }),
            expect
        );
        assert_eq!(keyed(Error::SecretOnCommandLine(key())), expect);
        let rejected = Error::Rejected(vec![
            FieldError {
                key: Some(key()),
                file: None,
                message: "no".into(),
            },
            FieldError {
                key: None,
                file: None,
                message: "unkeyed".into(),
            },
        ]);
        assert_eq!(keyed(rejected), expect);
        assert!(
            Error::Io(std::io::Error::other("x"))
                .field_errors()
                .is_empty()
        );
    }

    #[test]
    fn a_toml_error_names_the_place_not_the_text() {
        let text = "a = 1\ntoken = \"hunter2\" x\n";
        let err = text.parse::<toml::Table>().unwrap_err();
        let described = describe_toml_error(text, err.span(), err.message());
        assert!(described.starts_with("line 2, column "), "{described}");
        assert!(!described.contains("hunter2"), "{described}");
        let err = text.parse::<toml_edit::DocumentMut>().unwrap_err();
        let described = describe_toml_error(text, err.span(), err.message());
        assert!(described.starts_with("line 2, column "), "{described}");
        assert!(!described.contains("hunter2"), "{described}");
    }
}
