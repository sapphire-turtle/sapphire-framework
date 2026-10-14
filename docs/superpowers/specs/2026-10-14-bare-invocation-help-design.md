# The bare invocation prints the help

- PR: #259
- Crates: `sapphire-framework-server` (`FrameworkCommand`), `sapphire-framework-bridge`
  (`BridgeCommand`), `bridge/` (the `sapphire-bridge` binary)
- Revises: [`2026-09-24-app-command-system-design.md`](./2026-09-24-app-command-system-design.md)
  decision 1 ("bare invocation == serve") and its bridge edition in
  [`2026-09-26-app-command-system-phase2-plan.md`](../plans/2026-09-26-app-command-system-phase2-plan.md)
- Status: accepted

## Why

Decision 1 of the app command system made `<app>` with no subcommand the same thing as
`<app> serve`, and `sapphire-bridge` followed. That fit a world where the user started
the daemon by hand. Since then the server and the bridge run as OS services
(`service install` registers a unit that names `serve` explicitly), so a person typing the
bare command is almost never trying to start a daemon. They want to see what the command
can do.

Starting a server is also the wrong thing to do by accident. It holds the terminal in the
foreground, takes the IPC endpoint or the bridge's `InstanceLock`, and reports
`already running` when the service has it. None of that tells a newcomer about the other
verbs.

So: **the bare invocation prints the help; starting the server takes an explicit `serve`.**

## Decisions

1. **`sapphire-bridge` with no subcommand prints the help.** The subcommand is required
   (`command: BridgeCommand`, not `Option<BridgeCommand>`), and the parser carries
   `arg_required_else_help = true`, so clap prints the full help and exits with code 2
   (clap's code for a missing subcommand). `sapphire-bridge serve` is unchanged.

2. **No `Default` on the command enums.** `BridgeCommand` and `FrameworkCommand` no longer
   derive `Default`, and `Serve` is no longer `#[default]`. A default variant only existed to
   express "no subcommand = serve", and keeping it would invite an app to keep doing that
   with `unwrap_or_default()`.

3. **App CLIs follow the same convention.** The framework does not own an app's `Cli`
   struct, so it cannot enforce this. Its documented shape is a required subcommand plus
   `arg_required_else_help`:

   ```rust
   #[derive(Parser)]
   #[command(arg_required_else_help = true)]
   struct Cli {
       #[command(subcommand)]
       command: Command, // the app's verbs + `#[command(flatten)] Framework(FrameworkCommand)`
   }
   ```

   The parse tests (`bare_invocation_prints_the_help` in both crates) pin this shape.

4. **Service units are unaffected.** `AppServer::service_spec` and `bridge_service_spec`
   already start the executable with `["serve"]`, so an installed service never relied on
   the bare invocation and needs no reinstall.

## Compatibility

- Breaking for app crates: code that calls `FrameworkCommand::default()` or
  `Option<Command>::unwrap_or_default()` to start the server no longer compiles. The
  migration is decision 3's shape. journal, ledger, tally and sapphire-sync migrate in
  their own repositories.
- Breaking for anyone who started `sapphire-bridge` (or an app) by hand with no
  subcommand, e.g. in a script: add `serve`.
- The older specs and plans keep their "bare invocation == serve" wording as a record of
  what was decided then; this document is the one that applies.
