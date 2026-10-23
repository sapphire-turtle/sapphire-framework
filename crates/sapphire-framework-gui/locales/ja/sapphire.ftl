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
sapphire-new-not-empty = 空のフォルダーを選んでください（{ $path } は空ではありません）

# デバイスの行に出る役割。
sapphire-role-primary = プライマリ
sapphire-role-secondary = セカンダリ

# 埋め込み画面の状態行。`$slot` は `local` か `remote`、`$loaded` は `yes` か `no`、`$note` はブリッジ自身の文言です。
sapphire-embedding-active = { $slot ->
        [local] ローカル
       *[remote] リモート
    }モデル { $model }（次元数 { $dimension }）で埋め込み中です（{ $loaded ->
        [yes] 読み込み済み
       *[no] 未読み込み
    }）
sapphire-embedding-active-note = { $slot ->
        [local] ローカル
       *[remote] リモート
    }モデル { $model }（次元数 { $dimension }）で埋め込み中です（{ $loaded ->
        [yes] 読み込み済み
       *[no] 未読み込み
    }） — { $note }
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

# バックエンドやブリッジのエラーを、そのまま表示します。
sapphire-error-text = { $text }

# 複数の画面で使うボタン。
sapphire-action-add = 追加
sapphire-action-cancel = キャンセル
sapphire-action-clear = クリア
sapphire-action-copy = コピー
sapphire-action-create = 作成
sapphire-action-remove = 削除
sapphire-action-retire = 廃止
sapphire-action-save = 保存
sapphire-action-set = 設定

# 「<name> を入力して確認してください:」。`$name` は太字で表示します。「sapphire-devices-join-as」の `$name` も同じです。
sapphire-confirm-type = { $name } を入力して確認してください:

# 画面に表示できるものがないとき。
sapphire-unavailable-server = { $app } サーバーは利用できません。
sapphire-unavailable-bridge = ブリッジは利用できません。
sapphire-unavailable-hint = 「設定」→「About」を確認してください。

# 設定の一覧と About ページ。
sapphire-settings-general = 一般
sapphire-settings-appearance = 外観
sapphire-settings-sync = 同期
sapphire-settings-workspace = ワークスペース
sapphire-settings-devices = デバイス
sapphire-settings-workgroup = ワークグループ
sapphire-settings-external-devices = 外部デバイス
sapphire-settings-embedding = 埋め込み
sapphire-settings-about = About
sapphire-settings-version = バージョン { $version }
sapphire-settings-services = サービス

# 状態ドットにカーソルを合わせたときの文言。
sapphire-shell-connecting = 接続中…
sapphire-shell-healthy = サービスは稼働中です
sapphire-shell-update = アップデートがあります
sapphire-shell-problem = 確認が必要なサービスがあります

# サービスの状態行。
sapphire-banner-connecting = 接続中…
sapphire-banner-not-running = 停止中
sapphire-banner-incompatible = 互換性がありません: { $message }
sapphire-banner-error = エラー: { $message }
sapphire-banner-running = 稼働中 { $version }
sapphire-banner-sync-only = { $state } — 同期するときだけ必要です
sapphire-banner-install = サービスをインストールして起動
sapphire-banner-update = { $app } { $version } が公開されています。インストールしてから、サービスを再起動してください。
sapphire-banner-release-notes = リリースノート
sapphire-banner-by-hand = 手動でインストールするには:

# ワークスペース画面。
sapphire-workspaces-title = ワークスペース
sapphire-workspaces-none = ワークスペースがまだありません。既存のフォルダーを開くか、新しく作成するか、ワークグループから取り込んでください。
sapphire-workspaces-remove = 削除…
sapphire-workspaces-move = 移動…
sapphire-workspaces-move-title = { $name } の移動先を選んでください（フォルダー名は変わりません）
sapphire-workspaces-show-folder = フォルダーを開く
sapphire-workspaces-stop-sync = 同期を停止
sapphire-workspaces-start-sync = 同期を開始
sapphire-workspaces-switch = 別のワークスペースに切り替える
sapphire-workspaces-switch-hint = このデバイスで同期されるのは、選択中のワークスペースだけです。
sapphire-workspaces-open-folder = フォルダーを開く…
sapphire-workspaces-open-title = ワークスペースのフォルダーを開く
sapphire-workspaces-new = 新規…
sapphire-workspaces-new-title = 新しいワークスペース用の空のフォルダーを選んでください
sapphire-workspaces-new-name = 新しいワークスペースの名前
sapphire-workspaces-exists = { $name } は既に存在します
sapphire-workspaces-from-workgroup = ワークグループから
sapphire-workspaces-no-others = ワークグループに他のワークスペースはありません。
sapphire-workspaces-bring = このホストに取り込む…
sapphire-workspaces-bring-title = { $name } の置き場所を選んでください（その中に { $name } という名前のフォルダーを作成します）
sapphire-workspaces-remove-title = ワークスペースを削除
sapphire-workspaces-remove-body = { $name } はこのデバイスで同期されなくなります。他のデバイスのコピーは残り、ワークグループにも残ります。
sapphire-workspaces-delete-too = フォルダーとその中のすべてのファイルも削除する
sapphire-workspaces-remove-delete = フォルダーごと削除

# デバイス画面。
sapphire-devices-title = デバイス
sapphire-devices-invite = 招待…
sapphire-devices-invite-title = デバイスを招待
sapphire-devices-ticket = このチケットを新しいデバイスに渡してください。1 回だけ使えます。
sapphire-devices-join-as = 新しいデバイスでは、「{ $name }」という名前で参加してください
sapphire-devices-new-name = 新しいデバイスの名前
sapphire-devices-valid-for = 有効期間
sapphire-devices-create-ticket = チケットを作成
sapphire-devices-no-workgroup = このホストはまだワークグループに参加していません。ワークグループ画面を確認してください。
sapphire-devices-availability = 稼働率 { $tier }/3
sapphire-devices-availability-hover = 過去 1 週間のうち、このデバイスのブリッジが稼働していた割合です。3 は 99 % 以上、2 は 95 %、1 は 80 %、0 はそれ未満です。履歴が 1 週間に満たないデバイスは 1 段階低くなります。
sapphire-devices-this = （このデバイス）
sapphire-devices-priority = 優先度
sapphire-devices-retire-title = デバイスを廃止
sapphire-devices-retire-body = 廃止したデバイスは接続できなくなります。記録は残ります。

# 招待の有効期間。
sapphire-ttl-hour = 1 時間
sapphire-ttl-day = 24 時間
sapphire-ttl-week = 7 日間

# ワークグループ画面。
sapphire-workgroup-title = ワークグループ
sapphire-workgroup-ticket = チケット
sapphire-workgroup-paste = 貼り付け
sapphire-workgroup-invited-name = このデバイスが招待された名前
sapphire-workgroup-join = 参加
sapphire-workgroup-name = 名前
sapphire-workgroup-id = ID
sapphire-workgroup-devices = デバイス
sapphire-workgroup-this-device = このデバイス
sapphire-workgroup-create-heading = ワークグループを作成
sapphire-workgroup-device-name = このデバイスの名前
sapphire-workgroup-join-heading = チケットで参加

# 外部デバイス画面。
sapphire-external-title = 外部デバイス
sapphire-external-hint = 同期ではなくキーを使って、ワークグループのアプリにアクセスするクライアントです。
sapphire-external-found-first = 先にワークグループを作成するか、参加してください。
sapphire-external-too-old = このブリッジは古く、外部デバイスを管理できません。sapphire-bridge を更新してください。
sapphire-external-token = 「{ $name }」のトークンです。表示されるのは今回だけです。
sapphire-external-done = 完了
sapphire-external-none = 外部デバイスはありません。
sapphire-external-add-heading = 外部デバイスを追加
sapphire-external-name = 名前
sapphire-external-apps = アプリ
sapphire-external-note = メモ
sapphire-external-retired = 廃止済み
sapphire-external-save-apps = アプリを保存
sapphire-external-restore = 復元
sapphire-external-new-token = 新しいトークン

# 埋め込み画面。`$whose` は `device` か `workgroup` です。
sapphire-embedding-title = 埋め込み
sapphire-embedding-too-old = このブリッジは古く、埋め込みを設定できません。sapphire-bridge を更新してください。
sapphire-embedding-shadowed = このデバイス自身のモデル設定は無視されます。ワークグループの設定が適用されます。
sapphire-embedding-remote-heading = リモートモデル（{ $whose ->
        [workgroup] ワークグループ
       *[device] このデバイス
    }）
sapphire-embedding-remote-hint = OpenAI 互換のエンドポイントです。オンのときは最初に使われます。
sapphire-embedding-endpoint = エンドポイント
sapphire-embedding-model = モデル
sapphire-embedding-dimension = 次元数
sapphire-embedding-api-key = API キー
sapphire-embedding-key-is-set = 設定済み
sapphire-embedding-key-not-set = 未設定
sapphire-embedding-key-note = キーはこのデバイスにだけ保存され、同期されません。
sapphire-embedding-local-heading = ローカルモデル（{ $whose ->
        [workgroup] ワークグループ
       *[device] このデバイス
    }）
sapphire-embedding-local-hint = 各デバイスの CPU で計算します。使うには AVX2 が必要です。
sapphire-embedding-max-tokens = 最大トークン数
sapphire-embedding-on-this-device = このデバイスでは

# ファイルブラウザー。
sapphire-files-title = ファイル
sapphire-files-no-workspace = ワークスペースがまだありません。「設定」→「ワークスペース」で選んでください。
sapphire-files-back = ← ファイル
sapphire-files-select = ファイルを選ぶとプレビューを表示します。
sapphire-files-refresh = 更新
sapphire-files-empty = このフォルダーは空です。
sapphire-files-symlink-hover = シンボリックリンクです。たどりません。
sapphire-files-binary = バイナリファイル、{ $size } — プレビューはありません。
sapphire-files-truncated = 先頭の { $size } を表示しています。

# 非推奨のワークスペース一覧。
sapphire-manager-title = ワークスペース
sapphire-manager-open-existing = 既存を開く…
sapphire-manager-new = 新規
sapphire-manager-none = ワークスペースがまだありません。
sapphire-manager-none-hint = 新しく作成するか、既存のフォルダーを開いてください。
sapphire-manager-unreachable = アクセスできません
sapphire-manager-invalid = （無効: パスがありません）
sapphire-manager-delete = 削除
sapphire-manager-open = 開く
sapphire-manager-new-title = 新しいワークスペース
sapphire-manager-name = 名前
sapphire-manager-name-hint = 例: マイタイマー
sapphire-manager-remove-title = ワークスペースを削除
sapphire-manager-remove-body = ワークスペースを一覧から削除します。
sapphire-manager-remove-hint = ディスク上のファイルは削除されません。
sapphire-manager-pick-title = ワークスペースのフォルダーを開く
sapphire-manager-not-workspace = '{ $path }' は { $app } のワークスペースではありません
