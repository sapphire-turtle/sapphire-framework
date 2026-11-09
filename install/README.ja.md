# インストールスクリプト

> English: [README.md](README.md)

[`sapphire-bridge`](../bridge/headless/README.ja.md) と [`sapphire-sync`](../sync/README.ja.md) の
ヘッドレス版（CLI とサーバーを 1 つにしたバイナリ）を、このリポジトリの GitHub リリースから
インストールするスクリプトです。サーバー機などヘッドレスの環境向けです。sapphire の各アプリは、
同じ形のスクリプトをそれぞれのリポジトリに持っています（`https://github.com/sapphire-turtle/sapphire-<app>/tree/main/install`）。
アプリのスクリプトは bridge を入れないので、同期するホストでは bridge をここから入れてください。

デスクトップ版はこのスクリプトでは入りません。それぞれのリリースにインストーラーがあり、
CLI も含めて入り、どちらも自動で更新されます。

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
| `bridge` | `sapphire-bridge` |
| `sync` | `sapphire-sync` |

| `install.sh` | `install.ps1` | |
|---|---|---|
| `--dir <dir>` | `-Dir <dir>` | `~/.local/bin` の代わりに `<dir>` へ入れる |
| `--require-signature` | `-RequireSignature` | minisign が `PATH` にないとき、警告ではなく失敗にする |

- リリースがあるのは Linux x86_64 / aarch64、macOS aarch64、Windows x86_64 です。
- 入るのは最新版、つまりそのアプリのタグ接頭辞が付いたリリースのうち最大の `x.y.z` です（`update` コマンドと同じ規則）。
  あとからバージョンを変えるときは、バイナリ自身の `update` を使ってください。
- ダウンロードしたファイルは、インストールする前に次を確認します。
  - GitHub がリリースファイルに記録している SHA-256（Linux / macOS は `sha256sum` か `shasum`、Windows は `Get-FileHash`）。
  - [minisign](https://jedisct1.github.io/minisign/) が `PATH` にあれば、組織の鍵 `F4A73F39AA17C8EB`
    （`sapphire_framework_update::UPDATE_PUBKEY`。`update` が確認するのと同じ鍵）による署名（`<file>.sig`）。
    署名が対象のファイル名を指していることも確認します。

  minisign がないときは、警告して SHA-256 だけを確認します。`--require-signature` を付けると、そこで止まります。
  どちらかの確認に失敗したファイルや、確認する SHA-256・署名がないファイルはインストールしません。
- すでに `cargo install` や `cargo binstall`（Linux / macOS ではパッケージマネージャも）で入っているものには触れません。同じ方法で更新してください。
- デスクトップ版がすでに入っているアプリは飛ばします。デスクトップ版のパッケージに同じ CLI とサーバーが含まれ、
  二重に入れるとサーバーが 2 つ常駐するためです。判定は、Windows は `%LocalAppData%\sapphire-<app>-desktop\Update.exe`
  （パッケージの id。`%LocalAppData%\sapphire-<app>` はアプリのデータです）、macOS は `~/Applications` か
  `/Applications` の `Sapphire <App>.app`、Linux は AppImage を指す `~/.local/bin/sapphire-<app>` です。
  Linux のデスクトップ版は、頼まれたとき（Settings › About › Command line）にだけこのリンクを作るので、
  リンクを作っていない AppImage は見つけられません。1 台に入れるのは AppImage か CLI のどちらかにしてください。
- `install.ps1` はディレクトリをユーザーの `PATH` に追加します。`install.sh` は `PATH` にないときに知らせます。
- 以前のこのスクリプトは全アプリを扱い、どのアプリにも bridge を一緒に入れていました。今は自分のスクリプトを持つアプリ
  （`journal` など）を指定されると、そのスクリプトのコマンドを表示して止まります。`--no-bridge` は受け付けますが何もせず、`--version` はなくなりました。

## cargo で入れる

開発に使うマシンのように `PATH` で `~/.cargo/bin` が先に来る環境では、代わりに
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall) でそこへ入れてください。
同じリリースファイルをビルドせずにダウンロードし、`update` もそれを再実行するよう案内します。

```sh
cargo binstall sapphire-bridge-headless sapphire-sync-headless
```

## 各リポジトリのスクリプト

このスクリプトは、各アプリのものと同じく
[sapphire-turtle/.github の雛形](https://github.com/sapphire-turtle/.github/tree/main/templates/install)から作っています。
動きを変えるときは、まず雛形を直してから各リポジトリに反映します。違うのは冒頭の表だけです。
リリースファイルは[共通の命名](https://github.com/sapphire-turtle/.github#conventions)に従う必要があります。
