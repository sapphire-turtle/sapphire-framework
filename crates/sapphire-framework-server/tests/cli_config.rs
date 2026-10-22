//! `<app> config` (#314): the CLI against a running app server, and on the files when none
//! runs.
//!
//! Every command runs `server-test-app` as a process, the way a user runs an app's CLI, so
//! stdin, stdout, stderr and the exit code are the real ones. The server is that binary too
//! (`serve`); a bridge, when a test wants one, runs in this process on the same runtime
//! directory, where the server's standard connector finds it.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use sapphire_framework_bridge::{Bridge, BridgeDir, LoopbackNetwork, NetConfig, serve_restarting};
use sapphire_ipc::Endpoint;
use tokio::io::AsyncWriteExt;

const APP: &str = "sapphire-servertest";

/// How long a command or a start may take.
const DEADLINE: Duration = Duration::from_secs(60);

/// One host's directories: the runtime directory and the application's state.
struct Host {
    tmp: tempfile::TempDir,
}

/// What a command printed, and its exit code.
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

    fn runtime(&self) -> PathBuf {
        self.tmp.path().join("run")
    }

    fn state(&self) -> PathBuf {
        self.tmp.path().join("state")
    }

    /// The application's config directory: `<APP>` under the config root.
    fn config_dir(&self) -> PathBuf {
        self.state().join("config").join(APP)
    }

    fn command(&self, args: &[&str]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_server-test-app"));
        command
            .arg(self.runtime())
            .arg(self.state())
            .args(args)
            .kill_on_drop(true);
        command
    }

    /// Run `<app> args…`, with `stdin` on its standard input.
    async fn cli(&self, args: &[&str], stdin: Option<&str>) -> Run {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the CLI starts");
        let mut input = child.stdin.take().unwrap();
        if let Some(text) = stdin {
            input.write_all(text.as_bytes()).await.unwrap();
        }
        drop(input);
        let output = tokio::time::timeout(DEADLINE, child.wait_with_output())
            .await
            .expect("the CLI finished in time")
            .unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Start `<app> serve` and wait until it listens.
    async fn serve(&self) -> tokio::process::Child {
        let mut child = self
            .command(&["serve"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the server starts");
        let endpoint = Endpoint::in_dir(APP, self.runtime());
        let deadline = tokio::time::Instant::now() + DEADLINE;
        while !sapphire_ipc::probe(&endpoint).await.unwrap_or(false) {
            assert!(
                child.try_wait().unwrap().is_none(),
                "the server stopped before it listened"
            );
            assert!(tokio::time::Instant::now() < deadline, "never listened");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        child
    }

    /// Start a bridge on this host's runtime directory, where the server looks for one.
    ///
    /// Served as the real process serves it, so `bridge.restart` brings a new bridge up on
    /// the same listeners rather than ending this one. Wake-on-sync is off: the exe a route
    /// names would be `server-test-app`.
    async fn bridge(&self) -> tokio::task::JoinHandle<sapphire_framework_bridge::Result<()>> {
        let dir = BridgeDir::at(self.tmp.path().join("bridge")).unwrap();
        let control = Endpoint::in_dir("bridge", self.runtime());
        let data = Endpoint::in_dir("bridge-data", self.runtime());
        let net = LoopbackNetwork::new();
        let build = {
            let control = control.clone();
            move || {
                let bridge = Bridge::new(dir.clone(), Arc::new(net.transport(NODE)), "0.0.0").map(
                    |bridge| {
                        bridge
                            .net(NetConfig {
                                wake_on_sync: false,
                                discovery: false,
                                relays: Vec::new(),
                                use_default_relays: false,
                                ..NetConfig::default()
                            })
                            .control_endpoint(control.clone())
                            .data_endpoint(data.clone())
                    },
                );
                std::future::ready(bridge)
            }
        };
        let task = tokio::spawn(serve_restarting(build));
        let deadline = tokio::time::Instant::now() + DEADLINE;
        while !sapphire_ipc::probe(&control).await.unwrap_or(false) {
            assert!(!task.is_finished(), "the bridge stopped");
            assert!(tokio::time::Instant::now() < deadline, "no bridge");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        task
    }

    /// `config list`, once the server has reached the bridge: it connects on its own.
    async fn list_with_bridge(&self) -> Run {
        let deadline = tokio::time::Instant::now() + DEADLINE;
        loop {
            let list = self.cli(&["config", "list"], None).await;
            assert_ok(&list);
            if list.out.contains("net.") {
                return list;
            }
            assert!(tokio::time::Instant::now() < deadline, "{list:?}");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// The bridge's node id.
const NODE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn assert_ok(run: &Run) {
    assert_eq!(run.code, 0, "{run:?}");
}

/// The device file's text, or "" when there is none.
fn device_file(host: &Host) -> String {
    std::fs::read_to_string(host.config_dir().join("config.toml")).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_and_get_through_the_server() {
    let host = Host::new();
    let _server = host.serve().await;

    let set = host
        .cli(
            &["config", "set", "sapphire-servertest.greeting=hello"],
            None,
        )
        .await;
    assert_ok(&set);
    assert!(
        !set.out.contains("not running"),
        "went to the server: {set:?}"
    );
    assert!(!set.out.contains("restart required"), "a live key: {set:?}");

    let get = host
        .cli(&["config", "get", "sapphire-servertest.greeting"], None)
        .await;
    assert_ok(&get);
    assert_eq!(get.out.trim(), "sapphire-servertest.greeting = hello");

    // A key the server reads at start waits for its restart, and says whose.
    let set = host
        .cli(&["config", "set", "http.mcp.enabled=true"], None)
        .await;
    assert_ok(&set);
    assert!(
        set.out
            .contains("restart required: sapphire-servertest server"),
        "{set:?}"
    );
    assert!(!set.out.contains("sapphire-bridge"), "{set:?}");

    let unset = host
        .cli(&["config", "unset", "sapphire-servertest.greeting"], None)
        .await;
    assert_ok(&unset);
    let get = host
        .cli(&["config", "get", "sapphire-servertest.greeting"], None)
        .await;
    assert_eq!(get.out.trim(), "sapphire-servertest.greeting = <unset>");

    // With no bridge running, a key nobody here knows may be the bridge's.
    let get = host.cli(&["config", "get", "net.discovery"], None).await;
    assert_eq!(get.code, 1, "{get:?}");
    assert!(
        get.err.contains(
            "unknown setting net.discovery (or sapphire-bridge, which may know it, is not running)"
        ),
        "{get:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_list_shows_both_sides() {
    let host = Host::new();
    let _server = host.serve().await;

    // No bridge yet: the server's own keys, and the line that says the rest are missing.
    let list = host.cli(&["config", "list"], None).await;
    assert_ok(&list);
    assert!(list.out.contains("http.mcp.enabled = false"), "{list:?}");
    assert!(
        list.out
            .contains("sapphire-bridge is not running; its settings are not shown"),
        "{list:?}"
    );

    let _bridge = host.bridge().await;
    let list = host.list_with_bridge().await;
    assert!(list.out.contains("http.mcp.enabled = false"), "{list:?}");
    assert!(list.out.contains("update.check = true"), "{list:?}");
    assert!(!list.out.contains("not shown"), "{list:?}");

    // A scope lists what its file sets: the device's, on both sides.
    let set = host
        .cli(&["config", "set", "http.mcp.enabled=true"], None)
        .await;
    assert_ok(&set);
    let list = host.cli(&["config", "list", "--device"], None).await;
    assert_ok(&list);
    assert!(list.out.contains("http.mcp.enabled = true"), "{list:?}");
    assert!(!list.out.contains("not shown"), "{list:?}");

    // A bridge key goes to the bridge, and the restarts pending are named per process.
    let set = host
        .cli(&["config", "set", "net.discovery=false"], None)
        .await;
    assert_ok(&set);
    assert!(
        set.out.contains("restart required: sapphire-bridge"),
        "{set:?}"
    );
    assert!(
        set.out
            .contains("restart required: sapphire-servertest server"),
        "http.mcp.enabled still waits: {set:?}"
    );
    let get = host.cli(&["config", "get", "net.discovery"], None).await;
    assert_eq!(
        get.out.lines().next(),
        Some("net.discovery = false"),
        "{get:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_restart_restarts_the_app_server() {
    let host = Host::new();
    let mut server = host.serve().await;

    let set = host
        .cli(
            &["config", "set", "http.listen=127.0.0.1:0", "--restart"],
            None,
        )
        .await;
    assert_ok(&set);
    assert!(
        set.out.contains("the sapphire-servertest server restarted"),
        "{set:?}"
    );
    assert!(!set.out.contains("restart required"), "{set:?}");
    assert!(
        !set.out.contains("sapphire-bridge"),
        "no bridge key was pending: {set:?}"
    );

    // The same process serves again, with nothing left to restart.
    assert!(server.try_wait().unwrap().is_none(), "the server exited");
    let list = host.cli(&["config", "list"], None).await;
    assert_ok(&list);
    assert!(list.out.contains("http.listen = 127.0.0.1:0"), "{list:?}");
    assert!(!list.out.contains("restart required"), "{list:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_set_writes_the_file() {
    let host = Host::new();

    let set = host
        .cli(&["config", "set", "http.mcp.enabled=true"], None)
        .await;
    assert_ok(&set);
    assert!(
        set.out.contains(
            "the sapphire-servertest server is not running; it applies these settings when it starts"
        ),
        "{set:?}"
    );
    let text = device_file(&host);
    let table: toml::Table = text.parse().unwrap();
    assert_eq!(
        table["http"]["mcp"]["enabled"].as_bool(),
        Some(true),
        "{text}"
    );

    let get = host.cli(&["config", "get", "http.mcp.enabled"], None).await;
    assert_ok(&get);
    assert_eq!(get.out.trim(), "http.mcp.enabled = true");

    let list = host.cli(&["config", "list"], None).await;
    assert_ok(&list);
    assert!(list.out.contains("http.mcp.enabled = true"), "{list:?}");
    assert!(
        list.out
            .contains("sapphire-bridge is not running; its settings are not shown"),
        "{list:?}"
    );

    let unset = host
        .cli(&["config", "unset", "http.mcp.enabled"], None)
        .await;
    assert_ok(&unset);
    let get = host.cli(&["config", "get", "http.mcp.enabled"], None).await;
    assert_eq!(get.out.trim(), "http.mcp.enabled = false");

    // Nothing runs, so there is nothing to restart, and nothing is written.
    let set = host
        .cli(&["config", "set", "update.check=false", "--restart"], None)
        .await;
    assert_eq!(set.code, 1, "{set:?}");
    assert!(
        !device_file(&host).contains("check"),
        "{}",
        device_file(&host)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_config_migrates_server_toml() {
    let host = Host::new();
    let old = "[update]\ncheck = false\n";
    std::fs::write(host.config_dir().join("server.toml"), old).unwrap();

    let get = host.cli(&["config", "get", "update.check"], None).await;
    assert_ok(&get);
    assert_eq!(get.out.trim(), "update.check = false");
    assert_eq!(device_file(&host), old);
    assert!(host.config_dir().join("server.toml.v1").exists());
    assert!(!host.config_dir().join("server.toml").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_a_bridge_key_is_refused() {
    let host = Host::new();
    let expected = "the sapphire-servertest server is not running; start it, or use sapphire-bridge config for net.discovery";

    for args in [
        &["config", "set", "net.discovery=false"][..],
        &["config", "get", "net.discovery"][..],
        &["config", "unset", "net.discovery"][..],
    ] {
        let run = host.cli(args, None).await;
        assert_eq!(run.code, 1, "{args:?}: {run:?}");
        assert!(run.err.contains(expected), "{args:?}: {run:?}");
    }
    assert_eq!(device_file(&host), "", "nothing was written");
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_config_reports_an_unparsable_file() {
    let host = Host::new();
    let file = host.config_dir().join("config.toml");
    std::fs::write(&file, "this is = = not toml\n").unwrap();

    for args in [
        &["config", "list"][..],
        &["config", "set", "update.check=false"][..],
    ] {
        let run = host.cli(args, None).await;
        assert_ne!(run.code, 0, "{args:?}: {run:?}");
        assert!(
            run.err.contains(&file.display().to_string()),
            "the error names the file: {args:?}: {run:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "this is = = not toml\n"
    );

    // A file that parses but holds what the framework does not know is named too.
    std::fs::write(&file, "[http]\nnope = 1\n").unwrap();
    let run = host.cli(&["config", "list"], None).await;
    assert_ne!(run.code, 0, "{run:?}");
    assert!(run.err.contains(&file.display().to_string()), "{run:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_comes_from_stdin_only() {
    let host = Host::new();
    let secret = host
        .config_dir()
        .join("secrets")
        .join("sapphire-servertest.token");

    let run = host
        .cli(&["config", "set", "sapphire-servertest.token=x"], None)
        .await;
    assert_ne!(run.code, 0, "{run:?}");
    assert!(run.err.contains("is a secret"), "{run:?}");
    assert!(!secret.exists());

    let run = host
        .cli(
            &["config", "set", "sapphire-servertest.token", "--stdin"],
            Some("x\n"),
        )
        .await;
    assert_ok(&run);
    assert_eq!(std::fs::read_to_string(&secret).unwrap(), "x");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&secret).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Shown as set, never by value.
    let get = host
        .cli(&["config", "get", "sapphire-servertest.token"], None)
        .await;
    assert_eq!(get.out.trim(), "sapphire-servertest.token = <set>");
}

#[tokio::test(flavor = "multi_thread")]
async fn config_restart_restarts_the_bridge_then_the_app_server() {
    let host = Host::new();
    let _bridge = host.bridge().await;
    let mut server = host.serve().await;
    host.list_with_bridge().await;

    // One batch, a key of each process: both wait, and both restart, the bridge first.
    let set = host
        .cli(
            &[
                "config",
                "set",
                "net.discovery=false",
                "http.mcp.enabled=true",
                "--restart",
            ],
            None,
        )
        .await;
    assert_ok(&set);
    let bridge = set.out.find("sapphire-bridge restarted");
    let app = set.out.find("the sapphire-servertest server restarted");
    assert!(
        bridge.is_some() && app.is_some() && bridge < app,
        "the bridge restarts before the server: {set:?}"
    );
    assert!(!set.out.contains("restart required"), "{set:?}");

    assert!(server.try_wait().unwrap().is_none(), "the server exited");
    let list = host.list_with_bridge().await;
    assert!(list.out.contains("net.discovery = false"), "{list:?}");
    assert!(list.out.contains("http.mcp.enabled = true"), "{list:?}");
    assert!(!list.out.contains("restart required"), "{list:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_list_says_whether_the_bridge_runs() {
    let host = Host::new();
    let _bridge = host.bridge().await;

    let list = host.cli(&["config", "list"], None).await;
    assert_ok(&list);
    assert!(
        list.out
            .contains("start the sapphire-servertest server to see sapphire-bridge's settings"),
        "{list:?}"
    );
    assert!(!list.out.contains("not running; its settings"), "{list:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_workspace_scope_needs_a_selected_workspace() {
    let host = Host::new();
    let set = host
        .cli(
            &[
                "config",
                "set",
                "sapphire-servertest.greeting=hi",
                "--workspace",
            ],
            None,
        )
        .await;
    assert_ne!(set.code, 0, "{set:?}");
    assert!(set.err.contains("no workspace is selected"), "{set:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn update_goes_on_past_a_broken_config_file() {
    let host = Host::new();
    let file = host.config_dir().join("config.toml");
    std::fs::write(&file, "this is = = not toml\n").unwrap();

    // `--no-bridge`, and no releases named: nothing reaches the network.
    let run = host.cli(&["update", "--check", "--no-bridge"], None).await;
    assert!(
        run.err.contains("warning: could not read")
            && run.err.contains(&file.display().to_string()),
        "one warning naming the file: {run:?}"
    );
    assert_eq!(run.err.matches("warning").count(), 1, "{run:?}");
    assert!(
        run.out
            .contains("sapphire-servertest does not say where its releases are"),
        "update went on: {run:?}"
    );

    // `config` and `serve` still refuse the file.
    let list = host.cli(&["config", "list"], None).await;
    assert_ne!(list.code, 0, "{list:?}");
}
