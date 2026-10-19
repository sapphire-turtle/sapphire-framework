# Prometheus metrics on the server's HTTP listener

- PR: (to be opened)
- Crates: `sapphire-framework-server` (`ServerConfig`, `AppServer`, `http`),
  `sapphire-framework` (feature and re-export)
- Out of scope: `sapphire-bridge`. It has no HTTP listener of this kind.
- Status: accepted

## Why

An app server runs unattended as an OS service, often on a home server or a tailnet box.
Its owner can ask it how it is with `<app> status`, but nothing watches it over time. A
Prometheus scrape of the server answers two questions: is the process healthy (memory, CPU,
file descriptors, uptime), and is the framework's own work still happening (the bridge is
reachable, sync is not paused or failing, embedding is progressing, re-indexing runs).
Applications get a way to add metrics of their own.

The setting also brings the HTTP listener's settings together. Everything served over the
listener is configured under `[http]`, so `[mcp] http` moves to `[http.mcp] enabled`.

## Decisions

### 1. Configuration

```toml
[http]
listen = "127.0.0.1:3173"
allowed_hosts = ["box.tailnet.ts.net"]

[http.mcp]
enabled = true        # was [mcp] http; default false

[http.metrics]
enabled = true        # new; default false; serves GET /metrics
```

- `HttpConfig` gains `mcp: HttpMcpConfig { enabled: bool }` and
  `metrics: HttpMetricsConfig { enabled: bool }`, both `#[serde(default)]` and
  `deny_unknown_fields`. `ServerConfig::mcp` and `McpConfig` are removed.
- `listen` and `allowed_hosts` keep their names.
- **No compatibility for `[mcp]`.** `ServerConfig::load` checks for a top-level `mcp` table
  before deserializing. If it finds one, it returns
  `Error::Config("<path>: [mcp] http moved to [http.mcp] enabled")`, so the user is told
  where the setting went instead of seeing serde's generic unknown-field error. The
  CHANGELOG lists this under **Changed** as a breaking change, along with the type changes.

### 2. Features and dependencies

- `sapphire-framework-server` gains a feature
  `metrics = ["http", "dep:metrics-exporter-prometheus", "dep:metrics-process"]`.
  `sapphire-framework` forwards a feature of the same name.
- The `metrics` facade crate is a **non-optional** dependency of `sapphire-framework-server`.
  Without an installed recorder its macros do nothing, so the instrumented code needs no
  `#[cfg]`.
- `metrics-exporter-prometheus` is used without its HTTP server
  (`default-features = false`, keeping only what `PrometheusBuilder::install_recorder` and
  rendering need). The route is mounted on the existing axum listener.
- Instrumentation stays in `sapphire-framework-server`. Where the work happens in another
  crate (re-indexing in `sapphire-framework-workspace`), the server measures around its call
  into it. Other crates gain no dependency.
- With `[http.metrics] enabled = true` in a build without the feature, the server logs a
  warning and keeps starting, as it already does for MCP.

### 3. Routing

- `http::start` takes two routers: `protected`, which it wraps in
  `sapphire_framework_keys::protect` as today, and `public`, which it merges in unwrapped.
  `/metrics` is the only public route.
- **`/metrics` needs no token.** It carries nothing worth hiding (decision 5), and
  Prometheus scrapers expect an open endpoint. An authentication option is left until
  someone needs one.
- `allowed_hosts` (rmcp's `Host` allowlist) does not apply to `/metrics`. That check protects
  `/mcp` from DNS rebinding. `/metrics` is read-only and identifies nothing.
- The listener binds when any route is served. `[http.metrics] enabled = true` alone starts
  it. With neither `listen` nor an application default address, the existing
  "nothing to bind HTTP to" error is logged.

### 4. Recorder lifecycle

- When the feature is built and `[http.metrics] enabled` is true, `AppServer::run` installs
  the recorder with `PrometheusBuilder::new().install_recorder()`. The handle lives in a
  `static OnceLock<PrometheusHandle>`, so a second server in the same process (a test) reuses
  it.
- The server registers help texts (`describe_counter!`, `describe_gauge!`,
  `describe_histogram!`) for its metrics, and `metrics_process::Collector::describe`, at
  install time.
- If another recorder is already installed (the application installed its own), the server
  logs an error and does not mount `/metrics`. It keeps serving, as with any other HTTP
  failure.
- `GET /metrics` does the following in order:
  1. refreshes the framework's state gauges (decision 5)
  2. runs `metrics_process::Collector::collect`
  3. runs each application collector (decision 6)
  4. returns `handle.render()` with `Content-Type: text/plain; version=0.0.4`

  It answers 200 whatever the state. An unreachable bridge is a gauge value, not an error.

### 5. The metrics

**Naming and labels.** Every framework metric starts with `sapphire_`. Applications are told
apart by Prometheus' `job` label and `sapphire_build_info{app}`, not by metric names. Labels
take only bounded values: `app`, `version` and `result` (`ok` or `error`). **No metric or
label carries an identifier**: no workspace id, device id, path, workspace or device name, or
error text. Free-text states (`paused`, `last_error`) become 0/1 flags.

**Process** (`metrics-process`; a value the OS does not offer is left out):
`process_cpu_seconds_total`, `process_resident_memory_bytes`,
`process_virtual_memory_bytes`, `process_open_fds`, `process_max_fds`, `process_threads`,
`process_start_time_seconds`. In addition, `sapphire_build_info{app, version}` is 1.

**State gauges**, set on each scrape from state the server already holds. Sync state comes
from what `sync.status` answers for the current workspace, and update state from the update
check's shared status.

| Metric | Value |
|---|---|
| `sapphire_workspace_selected` | 1 if a workspace is selected |
| `sapphire_sync_enabled` | 1 if the workspace is synced |
| `sapphire_sync_bridge_available` | 1 if the bridge is reachable |
| `sapphire_sync_paused` | 1 if replication is paused |
| `sapphire_sync_error` | 1 if there is a `last_error` |
| `sapphire_sync_peers` | the workgroup's other devices |
| `sapphire_sync_live_sessions` | live sessions with peers now |
| `sapphire_embedding_vectors` | documents with a vector |
| `sapphire_embedding_pending` | documents still without one |
| `sapphire_embedding_running` | 1 while an embedding pass runs |
| `sapphire_update_available` | 1 if a newer release is known; absent when the check is off or has no answer yet |

Embedding gauges are absent when this host does not embed.

**Event metrics**, recorded where the event happens:

| Metric | Kind | What |
|---|---|---|
| `sapphire_ipc_connections` | gauge | open IPC connections (+1 on accept, −1 on end) |
| `sapphire_ipc_connections_total` | counter | IPC connections accepted |
| `sapphire_reindex_total{result}` | counter | re-index runs |
| `sapphire_reindex_duration_seconds` | histogram | re-index duration |
| `sapphire_sync_sessions_total{result}` | counter | sync sessions with a peer that ended |

The implementation plan decides the exact site of `sapphire_sync_sessions_total` (the end of
a live session) after reading the sync code. Requests to `/metrics` are not counted.

### 6. Application extension

- `sapphire_framework::metrics` re-exports the `metrics` crate, as `mcp::rmcp` re-exports
  rmcp. Applications use it for their own counters and histograms, so their facade version
  is always the server's.
- `AppServer::metrics_collector(Arc<dyn Fn() + Send + Sync>)` registers a function run on
  every scrape, for an application's state gauges. It can be called more than once, and the
  collectors run in registration order. It is synchronous, like `status_rows`.
- The rustdoc asks applications to prefix their metrics with their own name (for example
  `sapphire_journal_entries`) and to follow decision 5's label rule.

## Testing

- `config.rs` unit tests cover the new layout, the defaults (both off), and an old `[mcp]`
  table failing with the message that points to `[http.mcp] enabled`.
- An integration test binary of its own keeps the global recorder confined to it:
  - `/metrics` answers 200 without a token, while a protected route still answers 401
    without one.
  - The output contains `sapphire_build_info` and `sapphire_sync_enabled`.
  - A metric set by an application collector appears.
  - With metrics disabled there is no `/metrics` route.
  - The output contains neither the workspace root path nor its workspace id.

  The tests check that metrics are present and well-formed, not their exact values.

## Not in this change

- HTTP request metrics (counts, latency, status codes)
- An authentication option for `/metrics`
- Instrumentation inside `sapphire-framework-workspace`, `-retrieve` or `-sync`
- Applications' `server.toml` migration and their own metrics (follow-up PRs in their
  repositories)
