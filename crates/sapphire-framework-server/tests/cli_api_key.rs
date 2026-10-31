//! `<app> api-key` (#364): the served workspace's API keys, on the files, with no server
//! and no bridge; and the `external-device` it replaces.
//!
//! Every command runs `server-test-app` as a process, as `cli_config` does.

use std::path::PathBuf;
use std::time::Duration;

use sapphire_framework_keys::open_store;
use sapphire_framework_server::{Selection, SelectionFile};

const APP: &str = "sapphire-servertest";
const DEADLINE: Duration = Duration::from_secs(60);

struct Host {
    tmp: tempfile::TempDir,
}

#[derive(Debug)]
struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Host {
    fn new() -> Host {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("run")).unwrap();
        std::fs::create_dir_all(tmp.path().join("state/config").join(APP)).unwrap();
        Host { tmp }
    }

    /// Make a workspace and record it as the one this app serves.
    fn select(&self) -> PathBuf {
        let root = self.tmp.path().join("ws");
        std::fs::create_dir_all(root.join(format!(".{APP}"))).unwrap();
        let root = root.canonicalize().unwrap();
        SelectionFile::at(
            self.tmp
                .path()
                .join("state/config")
                .join(APP)
                .join("workspace.toml"),
        )
        .save(&Selection {
            root: root.clone(),
            sync: false,
        })
        .unwrap();
        root
    }

    async fn cli(&self, args: &[&str]) -> Run {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_server-test-app"))
            .arg(self.tmp.path().join("run"))
            .arg(self.tmp.path().join("state"))
            .args(args)
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(DEADLINE, output)
            .await
            .expect("the CLI finished in time")
            .unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

#[tokio::test]
async fn api_key_add_without_server() {
    let host = Host::new();
    let root = host.select();
    let run = host.cli(&["api-key", "add", "pendant"]).await;
    assert_eq!(run.code, 0, "{run:?}");
    let token = run.out.trim_end();
    assert!(!token.contains('\n'), "{run:?}");
    assert!(token.starts_with("sapphire-ak-"), "{run:?}");
    let store = open_store(APP, &root).unwrap();
    assert_eq!(store.authenticate(token).unwrap().name, "pendant");

    let run = host.cli(&["api-key", "list"]).await;
    assert_eq!(run.code, 0, "{run:?}");
    assert!(run.out.contains("pendant"), "{run:?}");
}

#[tokio::test]
async fn no_workspace_is_exit_1() {
    let host = Host::new();
    let run = host.cli(&["api-key", "list"]).await;
    assert_eq!(run.code, 1, "{run:?}");
    assert!(run.err.contains("workspace select"), "{run:?}");
}

#[tokio::test]
async fn external_device_alias_warns() {
    let host = Host::new();
    let root = host.select();
    let run = host.cli(&["external-device", "add", "pendant"]).await;
    assert_eq!(run.code, 0, "{run:?}");
    assert!(run.err.contains("renamed to `api-key`"), "{run:?}");
    let store = open_store(APP, &root).unwrap();
    assert!(store.authenticate(run.out.trim_end()).is_some(), "{run:?}");
}

#[tokio::test]
async fn external_device_app_is_refused() {
    let host = Host::new();
    let root = host.select();
    let run = host
        .cli(&[
            "external-device",
            "add",
            "pendant",
            "--app",
            "sapphire-other",
        ])
        .await;
    assert_ne!(run.code, 0, "{run:?}");
    assert!(run.err.contains("per workspace"), "{run:?}");
    assert!(open_store(APP, &root).unwrap().list().is_empty());
}

#[tokio::test]
async fn a_gone_workspace_is_exit_1_and_not_recreated() {
    let host = Host::new();
    let root = host.select();
    std::fs::remove_dir_all(&root).unwrap();
    let run = host.cli(&["api-key", "list"]).await;
    assert_eq!(run.code, 1, "{run:?}");
    assert!(run.err.contains("not reachable"), "{run:?}");
    assert!(!root.exists(), "the workspace must not be recreated");
}
