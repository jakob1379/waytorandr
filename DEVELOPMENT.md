# Development

## Environment

- Use the Nix dev shell for all local work: `nix develop`
- Run one-off commands through the shell with `nix develop -c <command>`
- The workspace is managed through `flake.nix`; prefer native Cargo commands inside the dev shell

## Normal Development Loop

- List shortcuts: `nix develop -c just`
- Format everything: `nix develop -c just fmt`
- Lint everything: `nix develop -c just lint`
- Format: `nix develop -c cargo fmt --all`
- Lint: `nix develop -c cargo clippy --all-targets --all-features`
- Test the full workspace: `nix develop -c cargo test -q`
- Test only the CLI crate: `nix develop -c cargo test -q -p waytorandr`

## CLI Integration Test

The full CLI integration test is debug-only. It uses a fake backend path compiled behind `debug_assertions` so the production build does not expose the test backend override.

- Run it through the repo script: `./scripts/test-cli-integration.sh`
- The script runs `cargo test -p waytorandr --test full_cli`, which builds and executes the debug `waytorandr` binary
- The test covers the real CLI process flow against simulated `wlroots`, `kscreen`, and `gnome` backends

## Packaging Check

- Build the package the same way Nix users will consume it: `nix build`
- Smoke-test the built binaries:
  - `./result/bin/waytorandr --help`
  - `./result/bin/waytorandr save --help`
  - `./result/bin/waytorandr service run --help`
  - `./result/bin/waytorandrd`

## Documentation Sync

- Keep `README.md` aligned with the generated CLI help text for command descriptions, `--default` wording, and `--json` support.
- Keep `README.md` aligned with setup-name/help wording on `save` and `status`.
- When JSON output changes, update both README examples/notes and the CLI integration tests in `bin/waytorandr/tests/full_cli.rs`.
