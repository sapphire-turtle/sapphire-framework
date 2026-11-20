# Sync delivery indicator in the shell's status bar

## Intent

A user who saves a ledger transaction, a journal entry, a tally or a timer edit and then
closes the laptop cannot tell whether the edit has left the device; today they wait an
arbitrary while. The server already knows (v0.29.0, #431: `BackendEvent::SyncDelivered`,
`SyncStatusResult::pending_delivery` / `acknowledging_peers`). Show it, so "safe to close"
is visible.

## Goals

- Every app on the framework's `Shell` (ledger, journal, tally, timer) shows, without
  app changes beyond a version bump, whether this device's edits have reached a peer.
- It reads at a glance: syncing / delivered (to whom, when) / stuck (no peer to deliver to).

## Non-goals

- An indicator beside each app's Save button (approach B) — a later step; tracked as an
  issue, not built here.
- sapphire-agent: it does not draw `Shell`, so it is untouched.
- Changing when the server notices an edit (watcher debounce 300 ms–1 s) or the server API.

## Anti-goals

- A green "delivered" shown while an edit is in fact pending, beyond the unavoidable
  watcher + refresh delay (≈3 s worst case). A stale ✓ must be recognisable as stale
  (it carries the time of delivery).
- Noise when sync is not enabled for the workspace: then nothing is shown.

## Approaches

- **A. Framework status bar (picked).** One change in sapphire-framework-gui; all apps
  get it. Not tied to a particular save; the status bar exists only when the window is
  wide, so narrow windows use the health dot.
- B. Beside each app's Save button. Tied to the save, but four repos, and several apps
  close the side pane on save, which hides the icon at once. Deferred (issue).
- A+B. Later, once B's issue is taken up.

## Design

**Data.** `FrameworkClient`'s background task already refreshes `server.info` every 2 s;
its `StatusReport.sync` carries `pending_delivery` and `acknowledging_peers`. In addition
the task subscribes to the app server's `workspace.subscribe` events and, on
`SyncDelivered`, records the peer's name and the time in the `Snapshot` and refreshes at
once, so ✓ appears without waiting for the next tick.

**States** (from the current workspace's sync status):

| Condition | Shown | Tone |
|---|---|---|
| sync not enabled / server down | nothing | — |
| `pending_delivery`, `acknowledging_peers > 0` | ⟳ "Syncing…" | neutral |
| `pending_delivery`, `acknowledging_peers == 0` | cloud-off "Not sent — no peer connected" | warning |
| not pending, a delivery seen | ✓ "Synced to *peer* · HH:MM" | success |
| not pending, none seen since start | ✓ "Synced" | success, weak |

`paused` / `last_error` stay with the existing health dot and status panel.

**Where.** Wide: in `Shell`'s status bar, at the right end just before the health dot,
after the app's own `status_ui`. Hover: a tooltip with the detail (peer, time, peers
connected). Narrow (no status bar): the health dot gets a ring in the indicator's tone
while an edit is pending; its tooltip says the same text.

**Text.** en and ja Fluent strings in sapphire-framework-gui's locales.

**Release.** A minor of sapphire-framework; the apps pick it up with their next bump.

## Left to the builder

- How the events subscription fits the client's cached connections (reconnect after the
  server restarts; a server without `SyncDelivered` just never sends it).
- Icons: corundum already has `cloud-off`, `refresh-cw`, `circle-check`.
- Exact layout and spacing; tests at the model level (state from a `Snapshot`) plus a
  render test like the existing shell tests.
