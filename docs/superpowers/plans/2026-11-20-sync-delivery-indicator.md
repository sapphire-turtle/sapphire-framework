# Sync delivery indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The framework `Shell`'s status bar shows whether this device's edits have reached a peer (Syncing / Not sent / ✓ Synced to *peer* · HH:MM), and the health dot carries a ring while an edit is pending.

**Architecture:** A pure model (`delivery.rs`) turns the existing 2 s `Snapshot` (whose `current.sync` has `pending_delivery` / `acknowledging_peers`) plus the last `SyncDelivered` event into a `Delivery` state. `FrameworkClient`'s background task subscribes to `workspace.subscribe` on its app connection and records each `SyncDelivered`. `shell.rs` draws the state.

**Tech Stack:** Rust 2024, egui (via corundum-framework-gui 0.13), tokio, jiff 1, Fluent (`locales/{en,ja}/sapphire.ftl`).

**Spec:** `docs/superpowers/specs/2026-11-20-sync-delivery-indicator-design.md`

## Global Constraints

- All changes in `crates/sapphire-framework-gui` (plus CHANGELOG). No server or API change.
- No breaking change to public types: `Snapshot`, `ServerState`, `ClientConfig` keep their fields (they are built literally in tests and possibly by apps).
- Sync not enabled for the current workspace, or server not up → nothing drawn, no ring.
- Every user-visible string in en and ja `.ftl`; tests use `test_support::english_ctx()`.
- Time shown as local `HH:MM` (jiff, `tz-system` feature added for this crate only).
- Icons from `corundum_framework_gui::icons`: `refresh_cw()`, `cloud_off()`, `circle_check()`.

## Review Focus

1. Server restarts while the window is open → the new connection is subscribed again; ✓ still appears for later deliveries. (Task 2 test `resubscribes_after_reconnect`.)
2. Workspace switched → a `SyncDelivered` recorded for the old root is not shown for the new one. (Task 1 test `delivery_of_another_root_is_ignored`.)
3. A new edit after a ✓ → state goes back to Syncing even though a `Delivered` is recorded. (Task 1 test `pending_overrides_last_delivery`.)
4. Sync paused while an edit is pending → "Not sent", not an endless "Syncing…" (my call, beyond the spec's table). (Task 1 test `paused_and_pending_is_stuck`.)
5. Narrow window (no status bar) → only the dot's ring, no text on the rail. (Task 3 test `narrow_shows_ring_not_text`.)

---

### Task 1: The delivery model

**Files:**
- Create: `crates/sapphire-framework-gui/src/delivery.rs`
- Modify: `crates/sapphire-framework-gui/src/lib.rs` (add `mod delivery; pub use delivery::{Delivered, Delivery};`)
- Modify: `crates/sapphire-framework-gui/Cargo.toml` (`jiff = { workspace = true, features = ["tz-system"] }`)

**Interfaces:**
- Produces:
  ```rust
  /// A `SyncDelivered` the client heard.
  #[derive(Clone, Debug, PartialEq)]
  pub struct Delivered { pub root: PathBuf, pub device_name: String, pub at: jiff::Timestamp }

  #[derive(Clone, Debug, PartialEq)]
  pub enum Delivery {
      /// An edit is pending and a peer that confirms is connected.
      Syncing,
      /// An edit is pending and nothing can take it: no confirming peer, or sync paused.
      Stuck,
      /// Nothing pending; the last delivery for this workspace, if one was heard.
      Synced(Option<Delivered>),
  }

  impl Delivery {
      /// `None` when the server is not up, serves no workspace, or does not sync it.
      pub fn of(snapshot: &Snapshot, last: Option<&Delivered>) -> Option<Delivery>;
  }
  ```

- [ ] **Step 1: Write the failing tests** in `delivery.rs` `mod tests`, building a `Snapshot` with `Conn::Up(ServerState { current: Some(CurrentWorkspace { root: "/ws".into(), sync: SyncStatusResult { enabled: true, pending_delivery, acknowledging_peers, paused, ..SyncStatusResult::not_synced() }, .. }), .. })` (copy the helper style of `health.rs` tests):
  - `nothing_without_sync`: `enabled: false` → `None`; `Conn::Absent` server → `None`; `current: None` → `None`.
  - `pending_with_a_peer_is_syncing`: pending, peers 1 → `Some(Syncing)`.
  - `pending_without_a_peer_is_stuck`: pending, peers 0 → `Some(Stuck)`.
  - `paused_and_pending_is_stuck`: pending, peers 1, `paused: Some("Offline".into())` → `Some(Stuck)`.
  - `pending_overrides_last_delivery`: pending, peers 1, `last = Delivered { root: "/ws", .. }` → `Some(Syncing)`.
  - `synced_carries_the_last_delivery`: not pending, last for `/ws` → `Some(Synced(Some(last)))`; no last → `Some(Synced(None))`.
  - `delivery_of_another_root_is_ignored`: not pending, last for `/old` → `Some(Synced(None))`.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui delivery` → fails to compile (`Delivery` missing).
- [ ] **Step 3: Implement** `Delivered`, `Delivery`, `Delivery::of` as above.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-gui delivery` → all pass.
- [ ] **Step 5: Commit** `feat(gui): a model of whether this device's edits have reached a peer`

### Task 2: Hear `SyncDelivered` in `FrameworkClient`

**Files:**
- Modify: `crates/sapphire-framework-gui/src/client/mod.rs` (field, accessor, listener in `run`)
- Modify: `crates/sapphire-framework-gui/src/client/conn.rs` (expose whether the app connection is new)
- Test: `crates/sapphire-framework-gui/src/client/mod.rs` `mod tests` (or `conn.rs` tests, reusing its `config()` and fake `Router` setup)

**Interfaces:**
- Consumes: `Delivered` (Task 1).
- Produces: `FrameworkClient::last_delivered(&self) -> Option<Delivered>` (clone of the latest heard).

Approach: `run` holds `Arc<Mutex<Option<Delivered>>>` shared with `FrameworkClient`. After each refresh, if `conns.app_client()` is `Some` and not `Arc::ptr_eq` with the client last subscribed, call `proto::SUBSCRIBE` on it (ignore an RPC error: an old server), take `client.notifications()` and spawn a task that, for each `Notification` with `method == proto::EVENT` whose params parse as `proto::EventParams` with `event: BackendEvent::SyncDelivered { device_name, .. }`, stores `Delivered { root: ws, device_name, at: jiff::Timestamp::now() }`, then wakes the loop through an `mpsc::UnboundedSender<()>` added as a third `select!` arm so the snapshot refreshes at once, and calls `repaint`. The task ends when the receiver closes (connection dropped); abort the previous one when re-subscribing. `Lagged` is skipped.

- [ ] **Step 1: Write the failing tests**, with a fake app server (`sapphire_ipc::serve` + `Router` answering `server.info`, `workspace.current`, `workspace.subscribe` with `Ack {}`, as `conn.rs` tests do) that, on subscribe, notifies one `workspace.event` with `EventParams { ws: "/ws".into(), event: BackendEvent::SyncDelivered { device_id: GrainId::random(), device_name: "laptop".into() } }`:
  - `hears_sync_delivered`: within 5 s, `client.last_delivered()` is `Some(d)` with `d.root == "/ws"` and `d.device_name == "laptop"`.
  - `resubscribes_after_reconnect`: stop the fake server, start a new one on the same endpoint whose event names `"phone"`; within 10 s `last_delivered().device_name == "phone"`.
  - `old_server_without_subscribe_is_fine`: router without `workspace.subscribe` → snapshot still `Conn::Up`, `last_delivered()` stays `None`.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui client::` → new tests fail.
- [ ] **Step 3: Implement** as described above.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-gui` → all pass.
- [ ] **Step 5: Commit** `feat(gui): the framework client hears when edits reach a peer`

### Task 3: Draw it in the shell

**Files:**
- Modify: `crates/sapphire-framework-gui/src/shell.rs` (`Frame::status_ui`, `Frame::foot_ui`, `status_dot`)
- Modify: `crates/sapphire-framework-gui/src/settings.rs` (`pub fn delivery(&self) -> Option<Delivery>` = `Delivery::of(&self.snapshot(), self.client.last_delivered().as_ref())`)
- Modify: `crates/sapphire-framework-gui/locales/en/sapphire.ftl`, `locales/ja/sapphire.ftl`
- Test: `shell.rs` `mod tests`

**Interfaces:**
- Consumes: `Delivery`, `Delivered` (Task 1), `FrameworkClient::last_delivered` (Task 2).

Placement: `Frame::status_ui` calls `self.app.status_ui(ui)` first, then, in a nested `Layout::right_to_left`, draws the indicator, so it sits at the right of the free space, just left of corundum's refresh button / health dot. `foot_ui` passes the delivery to `status_dot`, which strokes a 1.5 px ring of radius 6 in the tone when the state is `Syncing` or `Stuck`; its hover text appends the indicator's tooltip line. `status_ui` is only called when the status bar exists, so narrow gets the ring alone. The `Delivery` is computed once per frame in `Shell::show` and stored in `Frame`.

Strings (en / ja):

```
sapphire-sync-syncing = Syncing…               / 同期中…
sapphire-sync-stuck = Not sent — no peer connected / 未送信 — 接続中のピアなし
sapphire-sync-synced-to = Synced to { $peer } · { $time } / { $peer } に同期済み · { $time }
sapphire-sync-synced = Synced                  / 同期済み
sapphire-sync-tooltip-peers = { $count } peer(s) connected that confirm / 確認できるピア: { $count } 台
```

Tones: Syncing → `weak_text_color`; Stuck → `theme::Swatch::WARNING`; Synced → `Swatch::SUCCESS` (`Synced(None)` weak). Icons `refresh_cw` / `cloud_off` / `circle_check`, 14 px.

- [ ] **Step 1: Write the failing tests** (render with `english_ctx()` at a wide size `[1200.0, 800.0]` and at a narrow size `[400.0, 800.0]`, using a `Settings` whose client snapshot is injected — add a `#[cfg(test)]` `FrameworkClient::set_snapshot_for_test(Snapshot)` and `set_delivered_for_test(Option<Delivered>)` if no such hook exists):
  - `status_bar_says_syncing`: pending, peers 1 → texts contain `"Syncing…"`.
  - `status_bar_says_not_sent`: pending, peers 0 → contains `"Not sent — no peer connected"`.
  - `status_bar_names_the_peer`: not pending, last `Delivered { root: "/ws", device_name: "laptop", at }` → a text starts with `"Synced to laptop · "`.
  - `nothing_when_not_synced`: `enabled: false` → no text starting with `"Sync"` or `"Not sent"`.
  - `narrow_shows_ring_not_text`: narrow, pending → no `"Syncing…"` text drawn (ring is not asserted on pixels).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui shell::` → fail.
- [ ] **Step 3: Implement** as described; add the strings. Run the crate's i18n check test (`i18n_check`) to confirm en/ja keys match.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-gui && cargo clippy -p sapphire-framework-gui --all-features -- -D warnings` → pass.
- [ ] **Step 5: Manual check:** run sapphire-ledger desktop against this branch (`[patch.crates-io]` path override in a scratch copy, not committed), save a transaction with a peer connected and with none; see Syncing → ✓ and Not sent.
- [ ] **Step 6: Commit** `feat(gui): show on the status bar whether edits have reached a peer`; add a CHANGELOG `[Unreleased] ### Added` line in the same commit.
