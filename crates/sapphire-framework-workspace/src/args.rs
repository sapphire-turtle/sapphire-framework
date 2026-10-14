//! The shared workspace-directory CLI argument (issue #128), deprecated (#227).
//!
//! The app server serves one workspace, chosen with `workspace select` and kept
//! in its `workspace.toml`; a client command runs against that workspace over
//! IPC, so naming a directory on the client side has nothing to pick. Apps stop
//! flattening this into their CLI; it goes once they have.

#![allow(deprecated)]

use std::path::PathBuf;

/// Shared `--workspace-dir` argument: a workspace root named on the command line.
///
/// Deprecated: the server serves the workspace `workspace select` chose, and a
/// client command never names one.
#[deprecated(
    note = "the server serves the workspace `workspace select` chose; a client command does not name one"
)]
#[derive(clap::Args)]
pub struct WorkspaceArgs {
    /// Workspace root directory.
    #[arg(
        long = "workspace-dir",
        alias = "journal-dir",
        alias = "ledger-dir",
        alias = "data-dir",
        global = true,
        value_name = "DIR"
    )]
    pub workspace_dir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Probe {
        #[command(flatten)]
        args: WorkspaceArgs,
    }

    #[test]
    fn canonical_flag_and_legacy_aliases_all_parse() {
        for flag in [
            "--workspace-dir",
            "--journal-dir",
            "--ledger-dir",
            "--data-dir",
        ] {
            let parsed = Probe::try_parse_from(["app", flag, "/tmp/x"]).unwrap();
            assert_eq!(
                parsed.args.workspace_dir.unwrap(),
                std::path::PathBuf::from("/tmp/x"),
                "{flag}"
            );
        }
    }

    #[derive(Parser)]
    struct Sub {
        #[command(flatten)]
        args: WorkspaceArgs,
    }

    #[derive(clap::Subcommand)]
    enum Cmd {
        Sub(Box<Sub>),
    }

    #[derive(Parser)]
    struct App {
        #[command(subcommand)]
        cmd: Option<Cmd>,
    }

    #[test]
    fn global_flag_parses_after_a_subcommand() {
        let app = App::try_parse_from(["app", "sub", "--workspace-dir", "/tmp/y"]).unwrap();
        match app.cmd {
            Some(Cmd::Sub(sub)) => {
                assert_eq!(
                    sub.args.workspace_dir.unwrap(),
                    std::path::PathBuf::from("/tmp/y")
                )
            }
            None => panic!("subcommand was not parsed"),
        }
    }

    #[test]
    fn absent_flag_is_none() {
        assert!(
            Probe::try_parse_from(["app"])
                .unwrap()
                .args
                .workspace_dir
                .is_none()
        );
    }
}
