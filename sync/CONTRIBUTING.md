# Contributing

sapphire-sync's own rules. It lives in the framework's repository (#394), so
the repository's conventions — commit style, the doc-comment rule, the
English-code convention — are the [root CONTRIBUTING.md](../CONTRIBUTING.md)'s.
Link, don't copy.

## Layout

Under `sync/`, two editions, each one binary, over the same libraries:

- `headless/` — `sapphire-sync-headless`, the `sapphire-sync` binary without
  the GUI: a `main` that runs `sapphire-sync-cli`.
- `desktop/` — `sapphire-sync-desktop`, the GUI, which runs the same commands
  when given one.
- `crates/sapphire-sync-cli/` — the command line both editions run: a thin
  clap shell plus the app's own `sync` verb. Everything the framework provides
  stays the framework's; the app re-exports, never re-implements.
- `crates/sapphire-sync-server/` — the app server `serve` runs, and the status
  rows it reports.
- `crates/sapphire-sync-core/` — the core crate: the `CTX` static and the
  framework re-export. Keep it thin; it exists so the app has something of
  its own to depend on.

## Tests

Each crate's tests live in its own `tests/`: the command line's parse and
`sync` tests in `crates/sapphire-sync-cli/tests/`, the two-host sync scenarios
in `crates/sapphire-sync-server/tests/`, and the tests that run the real
`sapphire-sync` binary in `headless/tests/`. Top-level `tests/*.rs` files are
separate test crates; `crates/sapphire-sync-server/tests/e2e/common/` is a
shared harness included by path, not a crate of its own.

## CI

The repository's CI (`.github/workflows/ci.yml`) builds and tests sync with the
framework, so a framework change that breaks the app fails in the same PR. On
Linux x86_64 it then runs two guards for sync. The facade guard: outside
`[dev-dependencies]`, sync's crates depend on no framework crate but the
`sapphire-framework` facade (docs/ARCHITECTURE.md). The dependency guard — the default build's `cargo tree -p sapphire-sync-headless` must not
contain `fastembed`, `ort` or `ort-sys` (the embedding stack, the heavy part of
search). `sapphire-framework-retrieve` itself is expected: the framework's
workspace layer depends on it in every app, and without `fastembed-embed` it
brings only full-text search.

## What belongs here

This app is the reference: it exists to prove the framework's public surface
is enough. If a change here needs a new framework API, make it in the framework,
through the facade, in the same PR — not in the app — the app should stay as small as the framework
allows.
