## Dev

- run commands in the nix devshell with `nix develop -c <command>`
- Use the flake.nix to manage dependencies and build using native tooling
- always ensure projects build `nix build` after making changes
- keep `README.md` and `DEVELOPMENT.md` aligned with the current CLI help and JSON behavior when command surfaces change
- after CLI changes, verify `./result/bin/waytorandr --help`, `./result/bin/waytorandr save --help`, and `./result/bin/waytorandr service run --help`
