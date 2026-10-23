# sapphire-framework-gui's own strings. Keys are `sapphire-<area>-<what>`.

sapphire-settings = Settings

# The workspace's state badge (`views::model::Badge`). `$star` is `yes` or `no`; `$pending`
# is the files still to embed, 0 for none.
sapphire-badge-syncing = syncing · { $peers } { $peers ->
        [one] peer
       *[other] peers
    }{ $star ->
        [yes] { " · star" }
       *[no] { "" }
    }{ $pending ->
        [0] { "" }
       *[other] { " · embedding " }{ $pending }{ " pending" }
    }
sapphire-badge-paused = paused: { $reason }
sapphire-badge-error = error: { $reason }
sapphire-badge-not-synced = not synced
sapphire-badge-unreachable = unreachable

# Why a folder was refused for "Move…" and "New…".
sapphire-move-unmovable = { $path } cannot be moved
sapphire-move-exists = { $folder } already has a { $name }
sapphire-new-unreadable = { $path }: { $error }
sapphire-new-not-empty = Choose an empty folder ({ $path } is not empty)

# A device's roles, in its row.
sapphire-role-primary = primary
sapphire-role-secondary = secondary

# The embedding screen's status line. `$slot` is `local` or `remote`; `$loaded` is `yes` or
# `no`; `$note` is the bridge's own text.
sapphire-embedding-active = Embedding with the { $slot } model { $model } ({ $dimension } dimensions), { $loaded ->
        [yes] loaded
       *[no] not loaded
    }
sapphire-embedding-active-note = Embedding with the { $slot } model { $model } ({ $dimension } dimensions), { $loaded ->
        [yes] loaded
       *[no] not loaded
    } — { $note }
sapphire-embedding-unrunnable = The { $slot } model is configured, but this bridge cannot run it
sapphire-embedding-off = Embedding is off
sapphire-embedding-off-note = Embedding is off: { $note }
sapphire-switch-auto = Auto
sapphire-switch-on = On
sapphire-switch-off = Off
sapphire-auto-on = Auto: on
sapphire-auto-off-no-avx2 = Auto: off — this CPU has no AVX2

# A model form that does not check out. `$what` is `dimension` or `max-tokens`;
# `$error` is the bridge's own text.
sapphire-form-not-a-number = { $what ->
        [dimension] dimension
       *[max-tokens] max tokens
    }: `{ $value }` is not a number
sapphire-form-invalid = { $error }

# The file browser's errors. `$error` is the system's own text.
sapphire-files-symlink = { $path } is a symbolic link
sapphire-files-outside = { $path } is not a path inside the workspace
sapphire-files-io-error = { $error }

# An error from the backend or the bridge, shown as it came.
sapphire-error-text = { $text }

# Buttons used on several screens.
sapphire-action-add = Add
sapphire-action-cancel = Cancel
sapphire-action-clear = Clear
sapphire-action-copy = Copy
sapphire-action-create = Create
sapphire-action-remove = Remove
sapphire-action-retire = Retire
sapphire-action-save = Save
sapphire-action-set = Set

# "Type <name> to confirm:". `$name` is shown in bold, and so is the name in "join as".
sapphire-confirm-type = Type { $name } to confirm:

# Why a screen has nothing to show.
sapphire-unavailable-server = The { $app } server is not available.
sapphire-unavailable-bridge = The bridge is not available.
sapphire-unavailable-hint = See Settings → About.

# The Settings list and the About page.
sapphire-settings-general = General
sapphire-settings-appearance = Appearance
sapphire-settings-sync = Sync
sapphire-settings-workspace = Workspace
sapphire-settings-devices = Devices
sapphire-settings-workgroup = Workgroup
sapphire-settings-external-devices = External devices
sapphire-settings-embedding = Embedding
sapphire-settings-about = About
sapphire-settings-version = Version { $version }
sapphire-settings-services = Services

# The health dot's hover text.
sapphire-shell-connecting = Connecting…
sapphire-shell-healthy = Services running
sapphire-shell-update = An update is available
sapphire-shell-problem = A service needs attention

# The services' status line.
sapphire-banner-connecting = Connecting…
sapphire-banner-not-running = not running
sapphire-banner-incompatible = incompatible: { $message }
sapphire-banner-error = error: { $message }
sapphire-banner-running = running { $version }
sapphire-banner-sync-only = { $state } — needed only to sync
sapphire-banner-install = Install & start service
sapphire-banner-update = { $app } { $version } is available. Install it, then restart the service.
sapphire-banner-release-notes = Release notes
sapphire-banner-by-hand = To install by hand:

# The Workspace screen.
sapphire-workspaces-title = Workspace
sapphire-workspaces-none = No workspace yet. Open an existing folder, create a new one, or bring one from the workgroup.
sapphire-workspaces-remove = Remove…
sapphire-workspaces-move = Move…
sapphire-workspaces-move-title = Choose where to move { $name } (it keeps its folder name)
sapphire-workspaces-show-folder = Open folder
sapphire-workspaces-stop-sync = Stop sync
sapphire-workspaces-start-sync = Start sync
sapphire-workspaces-switch = Switch to another workspace
sapphire-workspaces-switch-hint = Only the selected workspace syncs on this device.
sapphire-workspaces-open-folder = Open folder…
sapphire-workspaces-open-title = Open a workspace folder
sapphire-workspaces-new = New…
sapphire-workspaces-new-title = Choose an empty folder for the new workspace
sapphire-workspaces-new-name = New workspace name
sapphire-workspaces-exists = { $name } already exists
sapphire-workspaces-from-workgroup = From the workgroup
sapphire-workspaces-no-others = No other workspaces in the workgroup.
sapphire-workspaces-bring = Bring to this host…
sapphire-workspaces-bring-title = Choose where to put { $name } (a folder named { $name } is created inside)
sapphire-workspaces-remove-title = Remove workspace
sapphire-workspaces-remove-body = { $name } stops syncing on this device. Other devices keep their copies, and it stays in the workgroup.
sapphire-workspaces-delete-too = Also delete the folder and every file in it
sapphire-workspaces-remove-delete = Remove and delete

# The Devices screen.
sapphire-devices-title = Devices
sapphire-devices-invite = Invite…
sapphire-devices-invite-title = Invite a device
sapphire-devices-ticket = Give this ticket to the new device. It works once.
sapphire-devices-join-as = On the new device, join as «{ $name }»
sapphire-devices-new-name = The new device's name
sapphire-devices-valid-for = Valid for
sapphire-devices-create-ticket = Create ticket
sapphire-devices-no-workgroup = This host has not joined a workgroup yet. See the Workgroup screen.
sapphire-devices-availability = availability { $tier }/3
sapphire-devices-availability-hover = How much of the last week this device's bridge was running: 3 is 99 % or more, 2 is 95 %, 1 is 80 %, 0 is less. A device with under a week of history is one lower.
sapphire-devices-this = (this device)
sapphire-devices-priority = priority
sapphire-devices-retire-title = Retire device
sapphire-devices-retire-body = A retired device can no longer connect. Its record stays.

# How long an invite lasts.
sapphire-ttl-hour = 1 hour
sapphire-ttl-day = 24 hours
sapphire-ttl-week = 7 days

# The Workgroup screen.
sapphire-workgroup-title = Workgroup
sapphire-workgroup-ticket = Ticket
sapphire-workgroup-paste = Paste
sapphire-workgroup-invited-name = The name this device was invited as
sapphire-workgroup-join = Join
sapphire-workgroup-name = Name
sapphire-workgroup-id = Id
sapphire-workgroup-devices = Devices
sapphire-workgroup-this-device = This device
sapphire-workgroup-create-heading = Create a workgroup
sapphire-workgroup-device-name = This device's name
sapphire-workgroup-join-heading = Join with a ticket

# The External devices screen.
sapphire-external-title = External devices
sapphire-external-hint = Clients that reach the workgroup's apps with a key instead of syncing.
sapphire-external-found-first = Found or join a workgroup first.
sapphire-external-too-old = This bridge is too old to manage external devices; update sapphire-bridge.
sapphire-external-token = The token for «{ $name }». It is shown only now.
sapphire-external-done = Done
sapphire-external-none = No external devices.
sapphire-external-add-heading = Add an external device
sapphire-external-name = Name
sapphire-external-apps = Apps
sapphire-external-note = Note
sapphire-external-retired = retired
sapphire-external-save-apps = Save apps
sapphire-external-restore = Restore
sapphire-external-new-token = New token

# The Embedding screen. `$whose` is `device` or `workgroup`.
sapphire-embedding-title = Embedding
sapphire-embedding-too-old = This bridge is too old to configure embedding; update sapphire-bridge.
sapphire-embedding-shadowed = This device's own model settings are ignored: the workgroup's apply.
sapphire-embedding-remote-heading = Remote model ({ $whose ->
        [workgroup] the workgroup
       *[device] this device
    })
sapphire-embedding-remote-hint = An OpenAI-compatible endpoint. Used first whenever it is on.
sapphire-embedding-endpoint = Endpoint
sapphire-embedding-model = Model
sapphire-embedding-dimension = Dimension
sapphire-embedding-api-key = API key
sapphire-embedding-key-is-set = set
sapphire-embedding-key-not-set = not set
sapphire-embedding-key-note = The key stays on this device; it is never synced.
sapphire-embedding-local-heading = Local model ({ $whose ->
        [workgroup] the workgroup
       *[device] this device
    })
sapphire-embedding-local-hint = Computed on each device's CPU; needs AVX2 to be usable.
sapphire-embedding-max-tokens = Max tokens
sapphire-embedding-on-this-device = On this device

# The file browser.
sapphire-files-title = Files
sapphire-files-no-workspace = No workspace yet — choose one in Settings → Workspace.
sapphire-files-back = ← Files
sapphire-files-select = Select a file to preview it.
sapphire-files-refresh = Refresh
sapphire-files-empty = This folder is empty.
sapphire-files-symlink-hover = A symbolic link: not followed.
sapphire-files-binary = Binary file, { $size } — no preview.
sapphire-files-truncated = Showing the first { $size }.

# The deprecated workspace list.
sapphire-manager-title = Workspaces
sapphire-manager-open-existing = Open Existing…
sapphire-manager-new = New
sapphire-manager-none = No workspaces yet.
sapphire-manager-none-hint = Create a new one or open an existing folder.
sapphire-manager-unreachable = unreachable
sapphire-manager-invalid = (invalid: no path)
sapphire-manager-delete = Delete
sapphire-manager-open = Open
sapphire-manager-new-title = New Workspace
sapphire-manager-name = Name
sapphire-manager-name-hint = e.g. My Timer
sapphire-manager-remove-title = Remove Workspace
sapphire-manager-remove-body = This removes the workspace from the list.
sapphire-manager-remove-hint = Files on disk are not deleted.
sapphire-manager-pick-title = Open workspace folder
sapphire-manager-not-workspace = '{ $path }' is not a { $app } workspace
