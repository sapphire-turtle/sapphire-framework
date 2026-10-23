# sapphire-framework-gui の文字列。キーは `sapphire-<area>-<what>` の形です。

sapphire-settings = 設定

# ワークスペースの状態バッジ。`$star` は `yes` か `no`、`$pending` は埋め込み待ちのファイル数です。
sapphire-badge-syncing = 同期中 · ピア { $peers } 台{ $star ->
        [yes] { " · スター" }
       *[no] { "" }
    }{ $pending ->
        [0] { "" }
       *[other] { " · 埋め込み待ち " }{ $pending }{ " 件" }
    }
sapphire-badge-paused = 一時停止中: { $reason }
sapphire-badge-error = エラー: { $reason }
sapphire-badge-not-synced = 未同期
sapphire-badge-unreachable = アクセスできません

# 「移動…」と「新規…」で選んだフォルダーを断る理由。
sapphire-move-unmovable = { $path } は移動できません
sapphire-move-exists = { $folder } には既に { $name } があります
sapphire-new-unreadable = { $path }: { $error }
sapphire-new-not-empty = 空のフォルダーを選んでください({ $path } は空ではありません)

# デバイスの行に出る役割。
sapphire-role-primary = プライマリ
sapphire-role-secondary = セカンダリ

# 埋め込み画面の状態行。`$slot` は `local` か `remote`、`$loaded` は `yes` か `no`、`$note` はブリッジ自身の文言です。
sapphire-embedding-active = { $slot ->
        [local] ローカル
       *[remote] リモート
    }モデル { $model }(次元数 { $dimension })で埋め込み中です。{ $loaded ->
        [yes] 読み込み済み
       *[no] 未読み込み
    }
sapphire-embedding-active-note = { $slot ->
        [local] ローカル
       *[remote] リモート
    }モデル { $model }(次元数 { $dimension })で埋め込み中です。{ $loaded ->
        [yes] 読み込み済み
       *[no] 未読み込み
    } — { $note }
sapphire-embedding-unrunnable = { $slot ->
        [local] ローカル
       *[remote] リモート
    }モデルは設定済みですが、このブリッジでは実行できません
sapphire-embedding-off = 埋め込みはオフです
sapphire-embedding-off-note = 埋め込みはオフです: { $note }
sapphire-switch-auto = 自動
sapphire-switch-on = オン
sapphire-switch-off = オフ
sapphire-auto-on = 自動: オン
sapphire-auto-off-no-avx2 = 自動: オフ — この CPU は AVX2 に対応していません

# モデルの入力が正しくないとき。`$what` は `dimension` か `max-tokens`、`$error` はブリッジ自身の文言です。
sapphire-form-not-a-number = { $what ->
        [dimension] 次元数
       *[max-tokens] 最大トークン数
    }: `{ $value }` は数値ではありません
sapphire-form-invalid = { $error }

# ファイルブラウザーのエラー。`$error` はシステム自身の文言です。
sapphire-files-symlink = { $path } はシンボリックリンクです
sapphire-files-outside = { $path } はワークスペース内のパスではありません
sapphire-files-io-error = { $error }
