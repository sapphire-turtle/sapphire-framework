# インストールスクリプト

> English: [README.md](README.md)

sapphire の各アプリを GitHub のリリースからインストールするスクリプトです。1 組で全アプリを扱い、
[`sapphire-bridge`](../bridge/README.ja.md) も一緒にインストール・更新します（アプリはホストの bridge を通じて同期するため）。

```sh
# Linux, macOS
curl -fsSL https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.sh | sh -s -- <app>...
```

```powershell
# Windows
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.ps1))) <app>...
```

| アプリ | 入るもの |
|---|---|
| `bridge` | `sapphire-bridge` のみ |
| `agent`, `agent-desktop` | `sapphire-agent`, `sapphire-agent-desktop` |
| `journal`, `journal-desktop` | `sapphire-journal`, `sapphire-journal-desktop` |
| `sync`, `sync-desktop` | `sapphire-sync`, `sapphire-sync-desktop` |

| `install.sh` | `install.ps1` | |
|---|---|---|
| `--dir <dir>` | `-Dir <dir>` | `~/.local/bin` の代わりに `<dir>` へ入れる |
| `--version <x.y.z>` | `-Version <x.y.z>` | 最新ではなく指定バージョンを入れる（アプリ 1 つのときのみ） |
| `--no-bridge` | `-NoBridge` | `sapphire-bridge` に触れない |

- リリースがあるのは Linux x86_64 / aarch64、macOS aarch64、Windows x86_64 です。
- 「最新」は、そのアプリのタグ接頭辞が付いたリリースのうち最大の `x.y.z` です（`update` コマンドと同じ規則）。
- すでに `cargo install`（Linux / macOS ではパッケージマネージャも）で入っているものには触れません。同じ方法で更新してください。
- `install.ps1` はディレクトリをユーザーの `PATH` に追加します。`install.sh` は `PATH` にないときに知らせます。

## アプリを追加するには

両方のスクリプト冒頭の表に、名前・リポジトリ・`-<os>-<arch>` より前のリリースファイル名・アプリの
`release-plz.toml` のタグ接頭辞を 1 行追加します。リリースファイルは[共通の命名](../ci/README.md#conventions)に従う必要があります。
