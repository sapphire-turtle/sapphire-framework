# インストールスクリプト

> English: [README.md](README.md)

sapphire の各アプリの CLI 版（CLI・app server・MCP サーバーを 1 つにしたバイナリ）を、GitHub のリリースから
インストールするスクリプトです。1 組で全アプリを扱い、[`sapphire-bridge`](../bridge/README.ja.md) も一緒にインストール・更新します
（アプリはホストの bridge を通じて同期するため）。サーバー機などヘッドレスの環境向けです。

デスクトップ版はこのスクリプトでは入りません。各アプリのリリース（`https://github.com/sapphire-turtle/sapphire-<app>/releases`）に
インストーラーがあり、CLI も含めて入り、どちらも自動で更新されます。

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
| `agent` | `sapphire-agent` |
| `journal` | `sapphire-journal` |
| `ledger` | `sapphire-ledger` |
| `sync` | `sapphire-sync` |
| `tally` | `sapphire-tally` |
| `timer` | `sapphire-timer` |

| `install.sh` | `install.ps1` | |
|---|---|---|
| `--dir <dir>` | `-Dir <dir>` | `~/.local/bin` の代わりに `<dir>` へ入れる |
| `--version <x.y.z>` | `-Version <x.y.z>` | 最新ではなく指定バージョンを入れる（アプリ 1 つのときのみ） |
| `--no-bridge` | `-NoBridge` | `sapphire-bridge` に触れない |

- リリースがあるのは Linux x86_64 / aarch64、macOS aarch64、Windows x86_64 です。
- 「最新」は、そのアプリのタグ接頭辞が付いたリリースのうち最大の `x.y.z` です（`update` コマンドと同じ規則）。
- すでに `cargo install` や `cargo binstall`（Linux / macOS ではパッケージマネージャも）で入っているものには触れません。同じ方法で更新してください。
- デスクトップ版がすでに入っているアプリは飛ばします。デスクトップ版のパッケージに同じ CLI とサーバーが含まれ、
  二重に入れるとサーバーが 2 つ常駐するためです。判定は、Windows は `%LocalAppData%\sapphire-<app>\Update.exe`、
  macOS は `~/Applications` か `/Applications` の `Sapphire <App>.app`、Linux は AppImage を指す
  `~/.local/bin/sapphire-<app>` です。
- `install.ps1` はディレクトリをユーザーの `PATH` に追加します。`install.sh` は `PATH` にないときに知らせます。

## cargo で入れる

開発に使うマシンのように `PATH` で `~/.cargo/bin` が先に来る環境では、代わりに
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall) でそこへ入れてください。
同じリリースファイルをビルドせずにダウンロードし、`update` もそれを再実行するよう案内します。

```sh
cargo binstall sapphire-bridge sapphire-journal
```

## アプリを追加するには

両方のスクリプト冒頭の表に、名前・リポジトリ・`-<os>-<arch>` より前のリリースファイル名・アプリの
`release-plz.toml` のタグ接頭辞を 1 行追加します。リリースファイルは[共通の命名](https://github.com/sapphire-turtle/.github#conventions)に従う必要があります。
アプリの README のインストール手順は、[共通の雛形](https://github.com/sapphire-turtle/.github/blob/main/templates/README-install.ja.md)に合わせます。
