//! The `embedding` subcommands, shared by the bridge's CLI and every app's.
//!
//! Both CLIs parse the same words into an [`EmbedRequest`]; what they do with it differs
//! (the bridge's CLI can write the files itself when no bridge runs), so only the parsing,
//! the key prompt and the printing live here.

use std::io::{BufRead, IsTerminal};

use crate::{
    ApiKey, ConfigEntry, DEFAULT_DIMENSION, DEFAULT_MAX_TOKENS, EmbedDeviceSetParams,
    EmbedModelSetParams, EmbedRequest, Entry, EntryValue, Kind, LOCAL_MODEL, LocalModel, Origin,
    RemoteModel, Scope, SecretState, SettingDef, Slot, SlotModel,
};

/// Show and change the embedding settings: a local and a remote model for the workgroup,
/// each switched per device, and this device's API key.
#[derive(Debug, clap::Subcommand)]
pub enum EmbeddingCommand {
    /// Show the settings, where they come from, and what this device does with them.
    Show,
    /// The local model, computed on each device's CPU.
    #[command(subcommand)]
    Local(LocalCommand),
    /// The remote model, an OpenAI-compatible endpoint.
    #[command(subcommand)]
    Remote(RemoteCommand),
    /// The remote model's API key on this device (never synced).
    #[command(subcommand)]
    Key(KeyCommand),
}

/// `embedding local …`.
#[derive(Debug, clap::Subcommand)]
pub enum LocalCommand {
    /// Configure the local model for the workgroup (options not given take their defaults).
    Set {
        /// Output dimension (64-2048).
        #[arg(long, default_value_t = DEFAULT_DIMENSION)]
        dimension: u32,
        /// Content tokens kept before embedding (1-8192).
        #[arg(long, default_value_t = DEFAULT_MAX_TOKENS)]
        max_tokens: usize,
    },
    /// Remove the local model from the workgroup.
    Clear,
    /// Switch the local model on this device.
    Device {
        /// `auto` is off on a CPU without AVX2, on otherwise.
        switch: Switch,
    },
}

/// `embedding remote …`.
#[derive(Debug, clap::Subcommand)]
pub enum RemoteCommand {
    /// Configure the remote model for the workgroup.
    Set {
        /// The endpoint's base URL (requests go to `<endpoint>/v1/embeddings`).
        #[arg(long)]
        endpoint: String,
        /// The model's name, as the endpoint knows it.
        #[arg(long)]
        model: String,
        /// Output dimension; longer vectors are cut to it.
        #[arg(long)]
        dimension: u32,
    },
    /// Remove the remote model from the workgroup.
    Clear,
    /// Switch the remote model on this device.
    Device {
        /// `auto` is on.
        switch: Switch,
    },
}

/// `embedding key …`.
#[derive(Debug, clap::Subcommand)]
pub enum KeyCommand {
    /// Store the key: read from standard input, or asked for without echo on a terminal.
    Set,
    /// Remove the key.
    Clear,
}

/// A slot's switch on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Switch {
    /// Decided by this device: the local model needs AVX2, the remote one is on.
    Auto,
    /// On.
    On,
    /// Off.
    Off,
}

impl Switch {
    fn enabled(self) -> Option<bool> {
        match self {
            Switch::Auto => None,
            Switch::On => Some(true),
            Switch::Off => Some(false),
        }
    }
}

impl EmbeddingCommand {
    /// The request these words make. `key set` reads the key here, through [`read_key`].
    pub fn request(self) -> std::io::Result<EmbedRequest> {
        let model_set = |slot, model| EmbedRequest::ModelSet(EmbedModelSetParams { slot, model });
        let device_set = |slot, switch: Switch| {
            EmbedRequest::DeviceSet(EmbedDeviceSetParams {
                slot,
                enabled: switch.enabled(),
            })
        };
        Ok(match self {
            EmbeddingCommand::Show => EmbedRequest::Show,
            EmbeddingCommand::Local(LocalCommand::Set {
                dimension,
                max_tokens,
            }) => model_set(
                Slot::Local,
                Some(SlotModel::Local(LocalModel {
                    model: LOCAL_MODEL.to_owned(),
                    dimension,
                    max_tokens,
                })),
            ),
            EmbeddingCommand::Local(LocalCommand::Clear) => model_set(Slot::Local, None),
            EmbeddingCommand::Local(LocalCommand::Device { switch }) => {
                device_set(Slot::Local, switch)
            }
            EmbeddingCommand::Remote(RemoteCommand::Set {
                endpoint,
                model,
                dimension,
            }) => model_set(
                Slot::Remote,
                Some(SlotModel::Remote(RemoteModel {
                    endpoint,
                    model,
                    dimension,
                })),
            ),
            EmbeddingCommand::Remote(RemoteCommand::Clear) => model_set(Slot::Remote, None),
            EmbeddingCommand::Remote(RemoteCommand::Device { switch }) => {
                device_set(Slot::Remote, switch)
            }
            EmbeddingCommand::Key(KeyCommand::Set) => EmbedRequest::KeySet(read_key()?),
            EmbeddingCommand::Key(KeyCommand::Clear) => EmbedRequest::KeyClear,
        })
    }
}

/// Read an API key: without echo on a terminal, else one line of standard input.
///
/// Never from an argument, which would leave the key in the shell's history and in the
/// process list.
pub fn read_key() -> std::io::Result<ApiKey> {
    let key = if std::io::stdin().is_terminal() {
        ApiKey::new(rpassword::prompt_password("API key: ")?)
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        ApiKey::new(line)
    };
    if key.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no key was given",
        ));
    }
    Ok(key)
}

/// `config …`: the registry's settings, by key.
#[derive(Debug, clap::Subcommand)]
pub enum ConfigCommand {
    /// List settings and their values.
    List {
        /// Which scope's file to list.
        #[command(flatten)]
        scope: ScopeFlags,
        /// Say where each value comes from.
        #[arg(long)]
        show_origin: bool,
    },
    /// Show one setting.
    Get {
        /// The setting's key.
        key: String,
    },
    /// Set settings: `key=value`..., or one secret key alone (prompted, or `--stdin`).
    Set {
        /// `key=value` pairs; a secret's key is given alone.
        #[arg(required = true)]
        entries: Vec<String>,
        /// Which scope's file to write (default: each key's own).
        #[command(flatten)]
        scope: ScopeFlags,
        /// Read a secret from standard input even on a terminal.
        #[arg(long)]
        stdin: bool,
        /// Restart the running bridge afterwards, and wait until it is back.
        #[arg(long)]
        restart: bool,
    },
    /// Remove settings (or every setting under a prefix), going back to the defaults.
    Unset {
        /// The keys, or prefixes of several keys.
        #[arg(required = true)]
        keys: Vec<String>,
        /// Which scope's file to remove from (default: each key's own).
        #[command(flatten)]
        scope: ScopeFlags,
        /// Restart the running bridge afterwards, and wait until it is back.
        #[arg(long)]
        restart: bool,
    },
}

/// The scope flags of `config`. There is no `--workspace`: an application adds its own.
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct ScopeFlags {
    /// This device's file.
    #[arg(long, conflicts_with = "workgroup")]
    device: bool,
    /// The workgroup's file.
    #[arg(long)]
    workgroup: bool,
}

impl ScopeFlags {
    /// The scope the flags name; `None` when neither is given.
    pub fn scope(&self) -> Option<Scope> {
        if self.device {
            Some(Scope::Device)
        } else if self.workgroup {
            Some(Scope::Workgroup)
        } else {
            None
        }
    }
}

/// Parse the `entries` of `config set` against `schema`.
///
/// `key=value` is read as the key's kind. A secret never takes its value from the command
/// line: it is given as a lone key, alone, and read through [`read_secret`] (`stdin`
/// forces standard input even on a terminal).
pub fn parse_entries(
    schema: &[SettingDef],
    entries: &[String],
    stdin: bool,
) -> Result<Vec<ConfigEntry>, String> {
    parse_entries_with(schema, entries, || {
        let tty = !stdin && std::io::stdin().is_terminal();
        read_secret(std::io::stdin().lock(), tty).map_err(|e| e.to_string())
    })
}

fn parse_entries_with(
    schema: &[SettingDef],
    entries: &[String],
    secret: impl FnOnce() -> Result<String, String>,
) -> Result<Vec<ConfigEntry>, String> {
    let kind_of = |key: &str| {
        schema
            .iter()
            .find(|d| d.key == key)
            .map(|d| &d.kind)
            .ok_or_else(|| format!("unknown setting `{key}`"))
    };
    let mut secret = Some(secret);
    let mut parsed = Vec::new();
    for entry in entries {
        let Some((key, text)) = entry.split_once('=') else {
            let key = entry.as_str();
            if kind_of(key)? != &Kind::Secret {
                return Err(format!("`{key}` needs a value: write `{key}=<value>`"));
            }
            if entries.len() != 1 {
                return Err(format!("the secret `{key}` must be set on its own"));
            }
            let read = secret.take().expect("one entry");
            parsed.push(ConfigEntry {
                key: key.to_owned(),
                value: crate::Value::String(read()?),
            });
            continue;
        };
        let kind = kind_of(key)?;
        if kind == &Kind::Secret {
            return Err(format!(
                "`{key}` is a secret: give the key alone and pass the value with --stdin or at the prompt, not on the command line"
            ));
        }
        parsed.push(ConfigEntry {
            key: key.to_owned(),
            value: kind.parse(text).map_err(|e| format!("{key}: {e}"))?,
        });
    }
    Ok(parsed)
}

/// Read a secret: without echo on a terminal, else one line from `stdin`. A trailing
/// newline is dropped; nothing at all is an error.
pub fn read_secret(mut stdin: impl BufRead, tty: bool) -> std::io::Result<String> {
    let mut text = if tty {
        rpassword::prompt_password("Value: ")?
    } else {
        let mut line = String::new();
        stdin.read_line(&mut line)?;
        line
    };
    while text.ends_with('\n') || text.ends_with('\r') {
        text.pop();
    }
    if text.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no value was given",
        ));
    }
    Ok(text)
}

/// One line per entry, `key = value`; secrets show only `<set>` or `<unset>`. With
/// `show_origin` each line ends in the scope the value came from. A final line names the
/// keys that await a restart.
pub fn describe_entries(entries: &[Entry], show_origin: bool) -> Vec<String> {
    let mut lines = Vec::new();
    for entry in entries {
        let value = match &entry.value {
            EntryValue::Value(v) => v.to_string(),
            EntryValue::Secret(SecretState::Set) => "<set>".to_owned(),
            EntryValue::Secret(SecretState::Unset) | EntryValue::Unset => "<unset>".to_owned(),
        };
        let mut line = format!("{} = {value}", entry.key);
        if show_origin {
            match entry.origin {
                Origin::Scope(scope) => line.push_str(&format!(" ({scope})")),
                Origin::Default => line.push_str(" (default)"),
            }
        }
        if let Some(note) = &entry.note {
            line.push_str(&format!("  # {note}"));
        }
        lines.push(line);
    }
    let pending: Vec<&str> = entries
        .iter()
        .filter(|e| e.pending_restart)
        .map(|e| e.key.as_str())
        .collect();
    if !pending.is_empty() {
        lines.push(format!("restart required: {}", pending.join(", ")));
    }
    lines
}

/// Add, list, retire, restore and rotate the workgroup's external devices: clients that
/// reach its apps with a key instead of syncing.
#[derive(Debug, clap::Subcommand)]
pub enum ExternalDeviceCommand {
    /// List the external devices.
    List,
    /// Add one, and print its token — the only time it is shown.
    Add {
        /// Its name.
        name: String,
        /// An application it may use; repeat for several.
        #[arg(long = "app")]
        apps: Vec<String>,
        /// A note.
        #[arg(long)]
        description: Option<String>,
    },
    /// Retire one: its token stops working; the record stays.
    Retire {
        /// Its name or id.
        selector: String,
    },
    /// Bring a retired one back, with the token it had.
    Restore {
        /// Its name or id.
        selector: String,
    },
    /// Give one a new token, keeping its id; the old token stops working at once.
    Rotate {
        /// Its name or id.
        selector: String,
    },
    /// Set the applications one may use (none: it may use nothing).
    Apps {
        /// Its name or id.
        selector: String,
        /// The applications.
        apps: Vec<String>,
    },
}

impl ExternalDeviceCommand {
    /// The request these words make. An `add` that names no application gets
    /// `default_app` — the application whose CLI ran it — when there is one.
    pub fn request(self, default_app: Option<&str>) -> crate::ExternalDeviceRequest {
        use crate::{ExternalDeviceAddParams, ExternalDeviceRequest, ExternalDeviceSetAppsParams};
        match self {
            ExternalDeviceCommand::List => ExternalDeviceRequest::List,
            ExternalDeviceCommand::Add {
                name,
                mut apps,
                description,
            } => {
                if apps.is_empty()
                    && let Some(app) = default_app
                {
                    apps.push(app.to_owned());
                }
                ExternalDeviceRequest::Add(ExternalDeviceAddParams {
                    name,
                    description,
                    apps,
                })
            }
            ExternalDeviceCommand::Retire { selector } => ExternalDeviceRequest::Retire(selector),
            ExternalDeviceCommand::Restore { selector } => ExternalDeviceRequest::Restore(selector),
            ExternalDeviceCommand::Rotate { selector } => ExternalDeviceRequest::Rotate(selector),
            ExternalDeviceCommand::Apps { selector, apps } => {
                ExternalDeviceRequest::SetApps(ExternalDeviceSetAppsParams { selector, apps })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(clap::Parser)]
    struct Cli {
        #[command(subcommand)]
        command: EmbeddingCommand,
    }

    fn parse(args: &[&str]) -> EmbedRequest {
        let mut argv = vec!["x"];
        argv.extend_from_slice(args);
        Cli::parse_from(argv).command.request().unwrap()
    }

    #[test]
    fn local_set_fills_in_the_defaults() {
        let EmbedRequest::ModelSet(p) = parse(&["local", "set", "--dimension", "512"]) else {
            panic!("not a model set");
        };
        assert_eq!(p.slot, Slot::Local);
        assert_eq!(
            p.model,
            Some(SlotModel::Local(LocalModel {
                dimension: 512,
                ..LocalModel::default()
            }))
        );
    }

    #[test]
    fn remote_set_needs_every_field() {
        let argv = [
            "x",
            "remote",
            "set",
            "--endpoint",
            "https://e",
            "--model",
            "m",
        ];
        assert!(Cli::try_parse_from(argv).is_err());
    }

    #[test]
    fn device_switches_map_to_options() {
        let EmbedRequest::DeviceSet(p) = parse(&["remote", "device", "off"]) else {
            panic!("not a device set");
        };
        assert_eq!((p.slot, p.enabled), (Slot::Remote, Some(false)));
        let EmbedRequest::DeviceSet(p) = parse(&["local", "device", "auto"]) else {
            panic!("not a device set");
        };
        assert_eq!((p.slot, p.enabled), (Slot::Local, None));
    }

    #[test]
    fn clear_is_a_model_set_without_a_model() {
        let EmbedRequest::ModelSet(p) = parse(&["remote", "clear"]) else {
            panic!("not a model set");
        };
        assert_eq!((p.slot, p.model), (Slot::Remote, None));
    }

    fn def(key: &str, kind: Kind) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind,
            scopes: vec![Scope::Device],
            overridable: false,
            apply: crate::Apply::Live,
            default: None,
            section: "t".into(),
            doc: "t".into(),
        }
    }

    fn schema() -> Vec<SettingDef> {
        vec![
            def("net.discovery", Kind::Bool),
            def(
                "net.relays",
                Kind::List {
                    item: Box::new(Kind::Url),
                },
            ),
            def("embed.remote.key", Kind::Secret),
        ]
    }

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| s.to_string()).collect()
    }

    fn no_secret() -> Result<String, String> {
        panic!("no secret expected")
    }

    #[test]
    fn entries_parse_by_kind() {
        use crate::Value;
        let got = parse_entries_with(
            &schema(),
            &words(&["net.discovery=off", "net.relays=https://a,https://b"]),
            no_secret,
        )
        .unwrap();
        assert_eq!(got[0].value, Value::Bool(false));
        assert_eq!(
            got[1].value,
            Value::List(vec![
                Value::String("https://a".into()),
                Value::String("https://b".into())
            ])
        );
    }

    #[test]
    fn a_secret_value_on_the_command_line_is_refused() {
        let err = parse_entries(&schema(), &words(&["embed.remote.key=sk"]), true).unwrap_err();
        assert!(err.contains("--stdin"), "{err}");
    }

    #[test]
    fn a_secret_key_must_be_set_alone() {
        let err = parse_entries_with(
            &schema(),
            &words(&["embed.remote.key", "net.discovery=on"]),
            no_secret,
        )
        .unwrap_err();
        assert!(err.contains("on its own"), "{err}");
    }

    #[test]
    fn a_lone_secret_key_is_read_not_taken_from_the_line() {
        let got = parse_entries_with(&schema(), &words(&["embed.remote.key"]), || Ok("sk".into()))
            .unwrap();
        assert_eq!(got[0].value, crate::Value::String("sk".into()));
    }

    #[test]
    fn a_lone_plain_key_asks_for_a_value() {
        let err = parse_entries_with(&schema(), &words(&["net.discovery"]), no_secret).unwrap_err();
        assert!(err.contains("net.discovery=<value>"), "{err}");
    }

    #[test]
    fn an_empty_stdin_secret_is_refused() {
        let err = read_secret("\n".as_bytes(), false).unwrap_err();
        assert!(err.to_string().contains("no value was given"));
        assert_eq!(read_secret("sk-1\r\n".as_bytes(), false).unwrap(), "sk-1");
    }

    #[test]
    fn an_unknown_key_is_named() {
        let err = parse_entries_with(&schema(), &words(&["net.nope=1"]), no_secret).unwrap_err();
        assert!(err.contains("net.nope"), "{err}");
    }

    #[test]
    fn list_prints_secrets_as_set_or_unset_and_origins_on_request() {
        use crate::Value;
        let entry = |key: &str, value, origin, pending| Entry {
            key: key.into(),
            value,
            origin,
            pending_restart: pending,
            note: None,
        };
        let entries = vec![
            entry(
                "embed.remote.key",
                EntryValue::Secret(SecretState::Set),
                Origin::Scope(Scope::Workgroup),
                false,
            ),
            entry(
                "net.discovery",
                EntryValue::Value(Value::Bool(false)),
                Origin::Default,
                true,
            ),
            entry("net.relays", EntryValue::Unset, Origin::Default, false),
        ];
        assert_eq!(
            describe_entries(&entries, false),
            [
                "embed.remote.key = <set>",
                "net.discovery = false",
                "net.relays = <unset>",
                "restart required: net.discovery"
            ]
        );
        let shown = describe_entries(&entries, true);
        assert_eq!(shown[0], "embed.remote.key = <set> (workgroup)");
        assert_eq!(shown[1], "net.discovery = false (default)");
    }

    #[test]
    fn a_note_is_appended() {
        let entries = vec![Entry {
            key: "a".into(),
            value: EntryValue::Unset,
            origin: Origin::Default,
            pending_restart: false,
            note: Some("hm".into()),
        }];
        assert_eq!(describe_entries(&entries, false), ["a = <unset>  # hm"]);
    }

    #[derive(clap::Parser)]
    struct CCli {
        #[command(subcommand)]
        command: ConfigCommand,
    }

    #[test]
    fn scope_flags_exclude_each_other() {
        let ConfigCommand::List { scope, .. } =
            CCli::parse_from(["x", "list", "--workgroup"]).command
        else {
            panic!()
        };
        assert_eq!(scope.scope(), Some(Scope::Workgroup));
        assert!(CCli::try_parse_from(["x", "list", "--device", "--workgroup"]).is_err());
    }

    #[derive(clap::Parser)]
    struct XCli {
        #[command(subcommand)]
        command: ExternalDeviceCommand,
    }

    #[test]
    fn add_defaults_to_the_running_app_and_takes_several() {
        let parse = |args: &[&str], app| {
            let mut argv = vec!["x"];
            argv.extend_from_slice(args);
            XCli::parse_from(argv).command.request(app)
        };
        let crate::ExternalDeviceRequest::Add(p) = parse(&["add", "pendant"], Some("agent")) else {
            panic!("not an add")
        };
        assert_eq!(p.apps, vec!["agent"]);
        let crate::ExternalDeviceRequest::Add(p) =
            parse(&["add", "hook", "--app", "a", "--app", "b"], Some("agent"))
        else {
            panic!("not an add")
        };
        assert_eq!(p.apps, vec!["a", "b"], "named apps replace the default");
    }
}
