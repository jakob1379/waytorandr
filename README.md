# waytorandr

`waytorandr` is a Wayland-native display profile manager inspired by `autorandr`.

## CLI

```text
Save, set, and switch Wayland display layouts.

Usage: waytorandr <COMMAND>

Commands:
  set          Set a saved profile, virtual configuration, or default/matching profile
  save         Save the current compositor layout as a profile
  remove       Remove a saved profile
  cycle        Set the next saved profile
  list         List profiles matching the current topology by default
  current      Show the active or currently matched profile
  detected     Show detected outputs and current geometry
  help         Print this message or the help of the given subcommand(s)

Options:
  -h, --help   Print help
```

Run `waytorandr set --help` or `waytorandr save --help` for command-specific examples.

Use `--json` on any `waytorandr` command to emit machine-readable JSON on stdout.

## Daemon

`waytorandrd` watches output changes and reapplies the configured default profile for the current hardware setup, or the best matching saved profile when no setup default exists.

- use `waytorandr` for explicit save/set/list/current workflows
- use `waytorandrd` when you want automatic reapplication after dock/undock or output hotplug events
- the daemon uses the same backend-selection path as the CLI and writes runtime state under the same XDG state directory

## Status

- actively tested on Niri and KDE Plasma/KWin
- wlroots output-management path is implemented and working
- KDE Plasma/KWin support is implemented through KScreen
- GNOME support is implemented through Mutter DisplayConfig

## Important Limits

`common` is a clone layout, not native backend mirroring.

- `common` uses one shared mode for all outputs at `(0, 0)`
- `mirror` uses native backend mirroring at one shared mode
- `largest` uses native backend mirroring while keeping each output at its largest mode
- when native mirroring is unavailable, `mirror` and `largest` point you to `wl-mirror`

Reference:
- `https://github.com/swaywm/wlr-protocols/issues/101`

## Nix

```bash
nix build
nix develop
nix develop -c cargo test
./result/bin/waytorandr --help
./result/bin/waytorandrd
```

### Home Manager

The flake exports a Home Manager module at:

- `homeManagerModules.default`
- `homeManagerModules.waytorandr`

Implementation files:

- `nix/home-manager/waytorandr.nix`
- `nix/modules/home-manager.nix`

The current module scope is intentionally small:

- it exposes `services.waytorandr.*`
- `services.waytorandr.enable = true` installs the package in `home.packages`
- it creates a `systemd --user` service for `waytorandrd`
- it does not provide declarative profile management through Nix

Real profile/default management still happens through the CLI, for example:

- `waytorandr save work-dock`
- `waytorandr set --default work-dock`
- `waytorandr list`
- `waytorandr current`

Persisted data remains in the normal XDG locations:

- `$XDG_CONFIG_HOME/waytorandr/profiles.json`
- `$XDG_STATE_HOME/waytorandr/state.toml`

If you do not set the XDG variables explicitly, these usually resolve to:

- `~/.config/waytorandr/profiles.json`
- `~/.local/state/waytorandr/state.toml`

Minimal Home Manager flake example:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    waytorandr.url = "github:jakob1379/waytorandr";
  };

  outputs = { nixpkgs, home-manager, waytorandr, ... }: {
    homeConfigurations."alice" = home-manager.lib.homeManagerConfiguration {
      pkgs = import nixpkgs { system = "x86_64-linux"; };
      modules = [
        waytorandr.homeManagerModules.waytorandr
        {
          home.username = "alice";
          home.homeDirectory = "/home/alice";
          home.stateVersion = "24.11";

          services.waytorandr.enable = true;
        }
      ];
    };
  };
}
```

Session assumptions and caveats:

- the module assumes a Linux Home Manager setup with `systemd --user`
- the daemon is started as a user service and is tied to `services.waytorandr.systemdTarget`
- that target defaults to `config.wayland.systemd.target`, which is commonly `graphical-session.target`
- the service unit also sets `ConditionEnvironment=WAYLAND_DISPLAY`, so it only starts inside a live Wayland graphical session
- if your session does not provide Home Manager's Wayland session target wiring, set `services.waytorandr.systemdTarget` explicitly or start the daemon some other way

Backend expectations:

- backend selection is runtime detection, not a Nix module option
- wlroots compositors are the most direct path
- KDE Plasma/KWin support is implemented through KScreen
- GNOME support is implemented through Mutter DisplayConfig

Read `nix/modules/home-manager.nix` for the exact option surface and inline
descriptions.

Dynamic shell completion is built in. After enabling it for your shell, `waytorandr set <TAB>` and `waytorandr remove <TAB>` include saved profile names.

## Releases

`Cargo.toml` is the canonical version source for this repo.

- `[workspace.package].version` drives all workspace crate versions
- `flake.nix` reads the package version from `Cargo.toml`
- GitHub releases are still triggered by Git tags in the form `vX.Y.Z`

Release flow:

1. Update `[workspace.package].version` in `Cargo.toml`.
2. Commit the version bump.
3. Create a matching annotated tag such as `git tag -a v0.2.2 -m "v0.2.2"`.
4. Optionally verify locally with `bash scripts/check-release-version.sh v0.2.2`.
5. Push the commit and tag.

Tagged CI now fails early if the pushed tag does not match the Cargo workspace version.

## Project Note

This project has been heavily AI-assisted.

I do not claim expert Rust knowledge, so treat the code as pragmatic and evolving rather than polished Rust craftsmanship.

## License

MIT OR Apache-2.0
