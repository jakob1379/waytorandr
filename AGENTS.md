## Dev

- run commands in the nix devshell with `nix develop -c <command>`
- Use the flake.nix to manage dependencies and build using native tooling
- after making changes, run `nix develop -c just check` (native cargo build, tests, and help smoke test)
- run `nix develop -c just smoke-release` (`nix build`) only when changing `flake.nix`, `nix/`, or packaging, or before a release
- keep `README.md` and `DEVELOPMENT.md` aligned with the current CLI help and JSON behavior when command surfaces change
- after CLI changes, verify `./target/debug/waytorandr --help`, `./target/debug/waytorandr save --help`, and `./target/debug/waytorandr service run --help` (`just smoke` runs them)
