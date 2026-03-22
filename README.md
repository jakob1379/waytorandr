# waytorandr

`waytorandr` is a Wayland-native display profile manager inspired by `autorandr`.

It is focused on:
- saving and setting real output layouts
- applying quick virtual layouts like `horizontal`, `vertical`, and `common`
- switching layouts through a single CLI and daemon

## Status

This project is early and still rough.

- actively tested on Niri
- wlroots output-management path is implemented and working
- more compositor testing is still needed
- GNOME and KDE backends are not implemented yet

## Important Limits

`common` is not true display mirroring.

Current generic wlroots output-management protocols do not expose real physical output mirroring semantics, so `waytorandr` cannot implement true mirror mode portably today.

- `common` places all connected outputs at the same origin with a shared mode
- `mirror` is a reserved command that explains the limitation and points to `wl-mirror`
- for actual mirrored content today, use `wl-mirror`

Reference:
- `https://github.com/swaywm/wlr-protocols/issues/101`

## Nix Only

This repo assumes Nix usage.

Build:

```bash
nix build
```

Development shell:

```bash
nix develop
```

Run tests:

```bash
nix develop -c cargo test
```

Run the built binaries:

```bash
./result/bin/waytorandr --help
./result/bin/waytorandrd
```

## Shell Completion

Dynamic completions are built into `waytorandr`, including runtime completion of saved profile names for commands like `set` and `remove`.

Bash:

```bash
echo 'source <(COMPLETE=bash waytorandr)' >> ~/.bashrc
```

Zsh:

```bash
echo 'source <(COMPLETE=zsh waytorandr)' >> ~/.zshrc
```

Fish:

```bash
echo 'COMPLETE=fish waytorandr | source' >> ~/.config/fish/completions/waytorandr.fish
```

After reloading your shell, `waytorandr set <TAB>` and `waytorandr remove <TAB>` will include saved profile names.

## CLI Examples

Detect outputs:

```bash
waytorandr detected
```

Show current compositor layout:

```bash
waytorandr detected
```

Save the current layout as `default`:

```bash
waytorandr save
```

Save the current layout and mark it as the default profile for future matching:

```bash
waytorandr save --default
```

Save the current layout with an explicit name:

```bash
waytorandr save docked
```

Save a named layout and make that saved profile the default:

```bash
waytorandr save docked --default
```

Dry-run a saved profile:

```bash
waytorandr set docked --dry-run
```

Set the matching profile, or the configured default if nothing matches:

```bash
waytorandr set
```

Dry-run virtual layouts:

```bash
waytorandr set horizontal --dry-run
waytorandr set vertical --dry-run
waytorandr set common --dry-run
waytorandr set common --largest --dry-run
```

## Profile Storage

Profiles are stored under:

```text
$XDG_CONFIG_HOME/waytorandr/profiles/
```

State is stored under:

```text
$XDG_STATE_HOME/waytorandr/
```

## Project Note

This project has been heavily AI-assisted.

I do not claim expert Rust knowledge, so treat the code as pragmatic and evolving rather than polished Rust craftsmanship.

## License

MIT OR Apache-2.0
