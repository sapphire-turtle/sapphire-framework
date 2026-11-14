# The workspace's files in an explorer, with undo

Asked for on 2026-11-13. Builds on corundum-framework 0.12's workspace fs and journal (its
spec: corundum-framework's `docs/superpowers/specs/2026-11-06-workspace-fs-journal-design.md`).

## Why

sapphire-sync's Files screen is a placeholder. The person needs to see the synced folder,
and to move, rename and trash what is in it: files the app does not manage (a PDF, a
photo) live there too. A move made by mistake must be undoable.

corundum-framework 0.12 has all of it, made for both asteria and sapphire: the `fs.*` and
`journal.*` methods (`corundum-framework-fs`), the explorer that drives them
(`ExplorerDriver`), the previews over `fs.read`, and the history screen (`HistoryPage`).
What is left is sapphire's part: serving the methods over its IPC, calling them from its
GUI, and placing the screens.

## Decisions

1. **The app server serves `fs.*` and `journal.*`** next to `workspace.*`, on the current
   workspace, through `corundum_framework_fs::FsService::dispatch`. `workspace.*` stays as
   it is: the CLI, MCP and the apps use it.
2. **The undo history is in memory** (`MemoryStore`). A restart of the server forgets it,
   and so does a switch to another workspace (decided 2026-11-13).
3. **The trash is not synced.** corundum's `fs.trash` moves an entry to
   `<own folder>/trash/<id>/<path>`. The own folder is `.sapphire-local`, not `.<app>`:
   the sync filter syncs `.<app>/` and no other dot-name, so the trash stays on the device.
   To the other devices a trashed file is deleted, and an undo makes it again.
4. **Who does it.** Every IPC client is the same person on this device, so the actor's
   principal is `local` for all of them, and `agent` is true for the MCP server's client
   (kind `mcp`). Any session may undo another's batch; corundum's history screen asks
   first for a batch of another session.
5. **After a change, sync is told at once.** After an `fs.apply` that is not a dry run,
   and after `journal.undo`, `journal.redo` and `journal.undo_batch`, the server scans the
   workspace and asks for embeddings, as `workspace.write_file` does.
6. **The GUI calls the app server on its own connection,** not through
   `FrameworkClient`'s command queue: that queue runs one command at a time and a join can
   hold it for seconds, while the explorer lists and reads in parallel.
7. **The history is a Settings page** ("History"), which an app adds with
   `with_app_sections`. sapphire-sync adds it.
8. **`FileBrowser` stays for now.** The new explorer replaces it in sapphire-sync; its
   removal is its own change.

## Server (`sapphire-framework-server`)

- `fs_methods(current, router) -> Router` adds `fs.version`, `fs.list`, `fs.stat`,
  `fs.read`, `fs.apply`, `journal.undo`, `journal.redo`, `journal.undo_batch`,
  `journal.history` and `journal.status`. The server's router gets them next to
  `workspace_router`.
- It keeps the `FsService` of the current root, made on the first call and made again when
  the current root differs from the one it was made for. No workspace: the error
  `workspace.*` answers.
- `SapphireApp` is the `WorkspaceApp`: `own_folder()` is `.sapphire-local`; every other hook
  keeps corundum's default (nothing managed, every dot-name hidden).
- The actor comes from the connection's `ClientInfo::kind`.
- An `FsError` is sent as the JSON-RPC error with its `code()` and message.

## GUI (`sapphire-framework-gui`)

- **`IpcFsTransport`**, a `corundum_framework_gui::fs_client::FsTransport`: each call is
  spawned on the runtime and answered through a `Pending`. It connects to the app server's
  endpoint on the first call (`ClientConfig::endpoints`), keeps the connection, and makes
  a new one after a failure. A JSON-RPC error becomes `FsError::Rpc`, anything else
  `FsError::Transport`.
- **`WorkspaceExplorer`**, the screen: an `ExplorerDriver` over `IpcFsTransport`, named
  after the workspace's folder in the trail. It is made again when the snapshot's
  current workspace changes, and shows a hint when there is none. Its `detail_ui` draws
  the picked file's preview, for the shell's side pane.
- **`HistorySection`**, a `SettingsSection` around `HistoryPage` over the same transport,
  refreshed when the page is opened and when the workspace changes.
- The words are corundum's; the two labels ("Files", "History") are the framework's
  `.ftl`s, in English and Japanese.

## sapphire-sync (`sync/desktop`)

- The Files screen is `WorkspaceExplorer`; the side pane shows its preview.
- Settings gets the History page.
- The window opens on Files, no longer on Settings.

## Testing

- **Server**: over an in-process connection, on a temporary workspace: list; a
  move, then `journal.undo` puts it back; a trash lands under `.sapphire-local/trash/`,
  which the sync filter refuses; switching the workspace serves the new root with an
  empty history; no workspace is an error; the MCP client's actor is an agent.
- **GUI**: `IpcFsTransport` against a router served in process: a result, an RPC error,
  and a server absent at the first call that a later call reaches. `WorkspaceFiles` starts
  the views again, with a new origin, only when the root changes. The explorer and the
  history page themselves are corundum's, tested there.

## Not in this round

- Emptying `.sapphire-local/trash/`. Its entries stay until the person removes them.
- Watching the folder: the explorer lists again after its own changes only.
- The mobile app's screen, and `fs.*` over MCP.
