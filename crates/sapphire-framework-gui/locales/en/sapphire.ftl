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
