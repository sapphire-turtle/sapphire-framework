//! MCP from both ends: `/mcp` on the server's HTTP listener, and the tool context the `mcp`
//! subcommand dials the server with.
//!
//! The HTTP side is driven with raw requests in MCP's Streamable HTTP transport, so the
//! test pins what a client on the network sees: off unless `config.toml` says so, a token
//! before anything else, and the framework's tools reaching the server's workspace.
//!
//! Each test owns its `AppContext` (the directories are first-writer-wins per context).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_framework_keys::{AuthConfig, Authenticated, GrainId, Verdict, Verifier};
use sapphire_framework_server::mcp::{Error as McpError, ToolContext};
use sapphire_framework_server::{AppServer, HttpConfig, HttpMcpConfig, ServerConfig};
use sapphire_ipc::{Client, ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};
use serde_json::{Value, json};

static CTX_OFF: AppContext = AppContext::new("sapphire-mcptest-off");
static CTX_HTTP: AppContext = AppContext::new("sapphire-mcptest-http");
static CTX_DIAL: AppContext = AppContext::new("sapphire-mcptest-dial");

/// The one token the fake verifier accepts.
const TOKEN: &str = "sapphire-ed-good";

/// Accepts [`TOKEN`] and nothing else.
struct Fake;

#[async_trait::async_trait]
impl Verifier for Fake {
    async fn verify(&self, token: &str) -> Verdict {
        if token == TOKEN {
            Verdict::Accepted(Authenticated {
                id: GrainId::random(),
                name: "agent".into(),
            })
        } else {
            Verdict::Refused
        }
    }
}

/// The environment lock, held for a test's life: every test in this binary points its
/// context's directories through the process environment.
struct EnvLock(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

/// Point `ctx`'s directories at `tmp`, serialised on the environment with every other test
/// in this binary.
fn point_at(ctx: &'static AppContext, tmp: &std::path::Path) -> EnvLock {
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let upper = ctx
        .app_name
        .trim_start_matches("sapphire-")
        .replace('-', "_")
        .to_uppercase();
    for cat in ["CACHE", "DATA", "CONFIG"] {
        // SAFETY: serialised by the lock above, which the caller holds for the test.
        unsafe {
            std::env::set_var(
                format!("SAPPHIRE_{upper}_{cat}_DIR"),
                tmp.join(cat.to_lowercase()),
            )
        };
    }
    ctx.init(AppKind::Server);
    EnvLock(lock)
}

/// A port nothing listens on, for the moment.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn wait_until_listening(endpoint: &Endpoint) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(endpoint).await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never started listening"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn start(server: AppServer, endpoint: &Endpoint) -> tokio::task::JoinHandle<()> {
    let server = server.endpoint(endpoint.clone());
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    wait_until_listening(endpoint).await;
    task
}

async fn client(ctx: &'static AppContext, endpoint: &Endpoint) -> Client {
    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    connect_or_absent(endpoint, ctx.app_name, info)
        .await
        .unwrap()
        .unwrap()
        .0
}

async fn stop(ctx: &'static AppContext, endpoint: &Endpoint, task: tokio::task::JoinHandle<()>) {
    let c = client(ctx, endpoint).await;
    let _: Value = c
        .call(sapphire_ipc::SHUTDOWN_METHOD, json!({}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the server stopped")
        .unwrap();
}

/// A workspace for the server to serve, with one file in it.
async fn workspace_with_a_file(c: &Client, root: PathBuf) {
    std::fs::create_dir_all(&root).unwrap();
    let _: proto::WorkspaceInitResult = c
        .call(
            proto::WORKSPACE_INIT,
            proto::WorkspaceInitParams { dir: root },
        )
        .await
        .unwrap();
    let _: proto::Ack = c
        .call(
            proto::WRITE_FILE,
            proto::ContentParams {
                path: "notes/hello.md".into(),
                content: "hello from the workspace\n".into(),
            },
        )
        .await
        .unwrap();
}

/// One MCP client session over Streamable HTTP.
struct Session {
    http: reqwest::Client,
    url: String,
    token: Option<&'static str>,
    id: Option<String>,
    next: u64,
}

impl Session {
    fn new(port: u16, token: Option<&'static str>) -> Session {
        Session {
            http: reqwest::Client::new(),
            url: format!("http://127.0.0.1:{port}/mcp"),
            token,
            id: None,
            next: 1,
        }
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .post(&self.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(body);
        if let Some(token) = self.token {
            req = req.bearer_auth(token);
        }
        if let Some(id) = &self.id {
            req = req.header("mcp-session-id", id);
        }
        req
    }

    /// Send a request and return its JSON-RPC response, from a JSON body or an event
    /// stream alike.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut res = self.post(&body).send().await.unwrap();
        assert!(res.status().is_success(), "{method}: {}", res.status());
        if let Some(session) = res.headers().get("mcp-session-id") {
            self.id = Some(session.to_str().unwrap().to_owned());
        }
        // The response may arrive as one event of a stream that stays open: read until
        // the line carrying our id, rather than to the end.
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            for line in seen.lines() {
                let data = line.strip_prefix("data:").unwrap_or(line).trim();
                if let Ok(message) = serde_json::from_str::<Value>(data)
                    && message.get("id") == Some(&json!(id))
                {
                    return message;
                }
            }
            let chunk = tokio::time::timeout_at(deadline, res.chunk())
                .await
                .expect("a response in time")
                .unwrap()
                .unwrap_or_else(|| panic!("{method}: the stream ended without a response: {seen}"));
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }

    async fn initialize(&mut self) -> Value {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0.0.0"}
                }),
            )
            .await;
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let res = self.post(&note).send().await.unwrap();
        assert!(res.status().is_success(), "initialized: {}", res.status());
        result
    }

    /// Call a tool and return its text and whether it is an error.
    async fn call(&mut self, tool: &str, arguments: Value) -> (String, bool) {
        let res = self
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await;
        let result = &res["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{tool}: no text in {res}"))
            .to_owned();
        (text, result["isError"].as_bool().unwrap_or(false))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_over_http_is_off_unless_the_config_turns_it_on() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_OFF, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_OFF.app_name, tmp.path().join("run"));
    let port = free_port();

    // The application has a default address, and no config.toml says anything.
    let server = AppServer::new(&CTX_OFF, "0.0.0")
        .http_listen(([127, 0, 0, 1], port).into())
        .http_auth(AuthConfig::new(Arc::new(Fake)));
    let task = start(server, &endpoint).await;

    let refused = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .bearer_auth(TOKEN)
        .send()
        .await;
    assert!(
        refused.is_err(),
        "something answered on the HTTP port: {refused:?}"
    );
    stop(&CTX_OFF, &endpoint, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_over_http_takes_a_token_and_serves_the_framework_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_HTTP, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_HTTP.app_name, tmp.path().join("run"));
    let port = free_port();

    let config = ServerConfig {
        http: HttpConfig {
            listen: Some(([127, 0, 0, 1], port).into()),
            allowed_hosts: Vec::new(),
            mcp: HttpMcpConfig { enabled: true },
            ..HttpConfig::default()
        },
        ..ServerConfig::default()
    };
    let server = AppServer::new(&CTX_HTTP, "0.0.0")
        .config(config)
        .http_auth(AuthConfig::new(Arc::new(Fake)));
    let task = start(server, &endpoint).await;
    let c = client(&CTX_HTTP, &endpoint).await;
    workspace_with_a_file(&c, tmp.path().join("ws")).await;

    // No token, or the wrong one: refused before MCP sees anything.
    for token in [None, Some("sapphire-ed-bad")] {
        let res = Session::new(port, token)
            .post(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED, "{token:?}");
    }

    let mut session = Session::new(port, Some(TOKEN));
    let init = session.initialize().await;
    assert_eq!(
        init["result"]["serverInfo"]["name"],
        json!(CTX_HTTP.app_name),
        "{init}"
    );

    let tools = session.request("tools/list", json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"workspace_search"), "{names:?}");
    assert!(
        !names.contains(&"workspace_write_file"),
        "the write tools are an application's choice: {names:?}"
    );

    // The tools reach the server's workspace over the in-process IPC connection.
    let (text, error) = session
        .call("workspace_read_file", json!({"path": "notes/hello.md"}))
        .await;
    assert!(!error, "{text}");
    assert_eq!(text, "hello from the workspace\n");

    let (text, error) = session.call("workspace_show", json!({})).await;
    assert!(!error, "{text}");
    let shown: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(shown["workspace"]["reachable"], json!(true), "{text}");

    let (text, error) = session
        .call("workspace_list_dir", json!({"path": "notes"}))
        .await;
    assert!(!error, "{text}");
    assert!(text.contains("hello.md"), "{text}");

    // A failure is the tool's answer, not a broken session.
    let (text, error) = session
        .call("workspace_read_file", json!({"path": "notes/missing.md"}))
        .await;
    assert!(error, "reading a missing file succeeded: {text}");
    let (_, error) = session.call("workspace_read_file", json!({})).await;
    assert!(error, "a call without its arguments succeeded");

    drop(c);
    stop(&CTX_HTTP, &endpoint, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stdio_side_dials_the_server_when_a_tool_needs_it() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_DIAL, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_DIAL.app_name, tmp.path().join("run"));

    // Made before any server runs, as an agent starts its MCP servers when it starts.
    let ctx = ToolContext::dial(CTX_DIAL.app_name, "0.0.0", None, endpoint.clone());
    let absent = ctx
        .call::<_, proto::WorkspaceCurrentResult>(proto::WORKSPACE_CURRENT, json!({}))
        .await;
    assert!(matches!(absent, Err(McpError::NotRunning(_))), "{absent:?}");

    let task = start(AppServer::new(&CTX_DIAL, "0.0.0"), &endpoint).await;
    let current: proto::WorkspaceCurrentResult = ctx
        .call(proto::WORKSPACE_CURRENT, json!({}))
        .await
        .expect("the server, once it runs");
    assert!(current.workspace.is_none());
    stop(&CTX_DIAL, &endpoint, task).await;

    // The server restarted under the agent: the next call dials the new one.
    let task = start(AppServer::new(&CTX_DIAL, "0.0.0"), &endpoint).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match ctx
            .call::<_, proto::WorkspaceCurrentResult>(proto::WORKSPACE_CURRENT, json!({}))
            .await
        {
            Ok(_) => break,
            Err(err) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "never reached the restarted server: {err}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    stop(&CTX_DIAL, &endpoint, task).await;
}
