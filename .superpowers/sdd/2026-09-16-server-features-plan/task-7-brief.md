### Task 7: Update the CLI table and the architecture note

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/command.rs` (tests for the new subcommand)
- Modify: `docs/ARCHITECTURE.md`

- [ ] **Step 1: Extend the CLI parse test**

```rust
#[test]
fn the_log_subcommand_parses() {
    assert!(Probe::try_parse_from(["b", "log"]).is_ok());
    assert!(Probe::try_parse_from(["b", "log", "--follow"]).is_ok());
    assert!(Probe::try_parse_from(["b", "log", "--lines", "50"]).is_ok());
}
```

- [ ] **Step 2: Note the operational surface**

`docs/ARCHITECTURE.md`, after the bridge rows:

```markdown
> **bridge の可視化**: `<bridge dir>/status.json`（5 秒ごと + 変化時、アトミック書き込み）と
> `<bridge dir>/logs/node.log`（10 MiB × 3 でローテーション）。書き手は単一インスタンスロックが
> 保証する 1 プロセスのみなので、ログは再起動をまたいで連続する。`sapphire-bridge status` は
> 稼働中の bridge に問い合わせ、応答が無ければ `status.json` を読む。
```

- [ ] **Step 3: Run everything and commit**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
git add crates docs/ARCHITECTURE.md
git commit -m "docs(bridge): record the operational surface"
```

---

## What this plan does not cover

| | Left for |
|---|---|
| `service install` for the bridge and for app servers | step 10 |
| Removing `-rpc`, `-remote-*` and `-blob`, and the per-kind directory migration | step 11 |
| mDNS discovery of devices on the same network | iroh's discovery is configured in Task 3; a sapphire-specific layer on top is not planned |
| Chunked or resumable transfer of large files | deferred with the sync spec's §2.8, and it moves with `-sync`, not with this layer |
| Bandwidth limits and scheduling | nobody has asked; the hooks would go in `LivePeers::fan_out` |
