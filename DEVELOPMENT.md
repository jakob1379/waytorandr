# Development

## Environment

- Use the Nix dev shell for all local work: `nix develop`
- Run one-off commands through the shell with `nix develop -c <command>`
- The workspace is managed through `flake.nix`; prefer native Cargo commands inside the dev shell

## Normal Development Loop

- List shortcuts: `nix develop -c just`
- Enter the dev shell to install Git hooks automatically: `nix develop`
- Run all autofixable hooks on demand: `nix fmt`
- Local gate (native cargo build, tests, help smoke test): `nix develop -c just check`
- CI gate (`just check` plus the `nix build` package smoke test and the Vicinae extension build): `nix develop -c just ci`
- Format everything: `nix develop -c just fmt`
- Lint everything: `nix develop -c just lint`
- Format: `nix develop -c cargo fmt --all`
- Lint: `nix develop -c cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- Test the full workspace: `nix develop -c cargo test --locked --workspace -q`
- Test the subprocess CLI flow: `nix develop -c just test-cli`
- Test only the CLI crate: `nix develop -c cargo test -q -p waytorandr`

`just` is the source of truth for CI, and `just check` includes the subprocess
CLI integration suite. Day-to-day builds use native Cargo in the dev shell
(`just build`, `just smoke`); `nix build` is reserved for packaging and releases
(`just build-release`, `just smoke-release`). `git-hooks.nix` is the source of truth
for local Git hook configuration, and entering the dev shell installs the
generated hooks automatically. Local hooks cover cheap file hygiene,
formatting, workflow validation, spell checking, and a lockfile consistency
check via `cargo metadata --locked`.

## CLI Integration Test

The full CLI integration test is debug-only. It uses a fake backend path compiled behind `debug_assertions` so the production build does not expose the test backend override.

- Run it through the checked recipe: `nix develop -c just test-cli`
- The script runs `cargo test -p waytorandr --test full_cli`, which builds and executes the debug `waytorandr` binary
- The test covers the real CLI process flow against simulated `wlroots`, `kscreen`, and `gnome` backends

## Vicinae Extension

`apps/vicinae` is a Vicinae extension that drives the `waytorandr` CLI. Node comes from the dev shell.

- Install dependencies: `cd apps/vicinae && nix develop -c npm ci`
- Test and typecheck: `nix develop -c npm test` and `nix develop -c npm run typecheck` (typecheck needs the types `npm run build` generates)
- Load it into a running Vicinae with live reload: `nix develop -c npm run dev`
- Build it as Nix users get it, with tests and typecheck in the check phase: `nix build .#vicinae-extension` (also run by `just build-release`)

## Daemon Timing Diagnostics

Use the existing `waytorandrd` daemon debug logs to diagnose slow automatic
profile switching. The daemon emits elapsed-millisecond fields for watcher
polls, topology stability waits, profile selection, backend validation, backend
apply, retry attempts, and policy completion.

- Foreground run: `nix develop -c waytorandrd --verbose`
- User service run: set `RUST_LOG=waytorandrd=debug,waytorandr_core=debug` in the service environment, then inspect `journalctl --user -u waytorandrd.service -b`

## Packaging Check

- Build the package the same way Nix users will consume it: `nix develop -c just build-release`
- Smoke-test the built binaries with `nix develop -c just smoke-release`, or manually:
  - `./result/bin/waytorandr --help`
  - `./result/bin/waytorandr save --help`
  - `./result/bin/waytorandr set --help`
  - `./result/bin/waytorandr service run --help`
  - `./result/bin/waytorandrd --help`
  - `./result/bin/waytorandrd --verbose --help`

## Documentation Sync

- Keep `README.md` aligned with the generated CLI help text for command descriptions, explicit `set auto` wording, current-setup `remove` wording, `--default` wording, and `--json` support.
- Keep `README.md` aligned with setup-name/help wording on `save` and `status`.
- Keep the daemon section aligned with `waytorandrd --help` when daemon options change.
- When JSON output changes, update both README examples/notes and the CLI integration tests in `bin/waytorandr/tests/full_cli.rs`.
