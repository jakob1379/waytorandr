# waytorandr

`waytorandr` is a Wayland-native display profile manager inspired by `autorandr`.
It saves the current layout as a named profile, reapplies profiles on demand,
and can automatically restore the right layout when outputs change.

It includes:

- `waytorandr`: CLI for saving, applying, listing, and inspecting profiles
- `waytorandrd`: daemon for automatic re-application after dock, undock, or hotplug events

## Scope

- Save the current compositor layout as a named profile
- Reapply a saved profile or a built-in virtual layout
- Set a preferred default profile per hardware setup
- Automatically restore the best layout when outputs change
- Emit machine-readable JSON from most commands with `--json`
- Offer dynamic shell completion for saved profile names and `set` targets

## Backend Support

`waytorandr` chooses a backend at runtime based on the active session.

- `wlroots` compositors
- KDE Plasma via KScreen
- GNOME via Mutter DisplayConfig

Actively exercised on Niri and KDE Plasma/KWin.

> [!NOTE]
> The CLI and daemon are for Wayland sessions. The bundled user service starts
> only when `WAYLAND_DISPLAY` is present.

## Quick Start

Build the package:

```bash
nix build
./result/bin/waytorandr --help
```

Build distro-oriented release artifacts on `x86_64-linux`:

```bash
nix build .#deb
nix build .#apk
nix build .#aut       # Arch Linux package alias
nix build .#aur       # AUR-ready PKGBUILD and .SRCINFO
nix build .#rpm
nix build .#appimage
nix build .#flatpak
nix build .#snap
```

Inspect the current layout and save a profile:

```bash
./result/bin/waytorandr detected
./result/bin/waytorandr save work-dock
./result/bin/waytorandr set work-dock
./result/bin/waytorandr set --default work-dock
```

Preview a virtual layout without applying it:

```bash
./result/bin/waytorandr set horizontal --dry-run
./result/bin/waytorandr set mirror --dry-run
```

## CLI

```text
waytorandr set       Set a saved profile, virtual configuration, or default/matching profile
waytorandr save      Save the current compositor layout as a profile
waytorandr remove    Remove a saved profile
waytorandr cycle     Set the next saved profile
waytorandr list      List profiles matching the current topology by default
waytorandr current   Show the active or currently matched profile
waytorandr detected  Show detected outputs and current geometry
waytorandr version   Show the waytorandr version
waytorandr service   Manage the waytorandrd user service
```

Built-in `set` targets:

- `off`
- `common` - clone all connected outputs at the largest shared resolution
- `largest` - clone all connected outputs at the same origin using each output's largest mode
- `mirror` - native mirroring at one shared mode
- `horizontal`
- `vertical`

Run `waytorandr set --help` and `waytorandr save --help` for the full command
reference and examples.

> [!TIP]
> Use `--json` on supported commands when you want stable machine-readable output.

## Daemon

`waytorandrd` watches for topology changes and reapplies the configured default
profile for the current setup, or the best matching saved profile when no setup
default exists.

For non-Home-Manager setups, `waytorandr` can manage a user service directly:

```bash
waytorandr service install
waytorandr service start
waytorandr service status
```

`waytorandr service run` starts the daemon in the foreground and does not support `--json`.

## Home Manager

The flake exports:

- `homeManagerModules.default`
- `homeManagerModules.waytorandr`

Minimal flake example:

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

Current option surface:

- `services.waytorandr.enable`
- `services.waytorandr.package`
- `services.waytorandr.environment`
- `services.waytorandr.systemdTarget`

Profiles and runtime state remain in standard XDG paths:

- `$XDG_CONFIG_HOME/waytorandr/profiles.json`
- `$XDG_STATE_HOME/waytorandr/state.toml`

If the XDG variables are unset, these typically resolve to:

- `~/.config/waytorandr/profiles.json`
- `~/.local/state/waytorandr/state.toml`

## Limits

> [!IMPORTANT]
> `common` is not native mirroring. It chooses the largest shared resolution and
> places all outputs at `(0, 0)`.

- `mirror` uses native backend mirroring at one shared mode
- `largest` overlaps outputs at the same origin while keeping each output at its largest mode
- when a backend cannot satisfy `mirror`, `waytorandr` prints backend-specific guidance instead of sending an invalid layout

Reference:

- `https://github.com/swaywm/wlr-protocols/issues/101`

## Development

Use the Nix dev shell for local work:

```bash
nix develop -c cargo fmt --all
nix develop -c cargo test -q
nix build
```

More development details live in `DEVELOPMENT.md`.

## License

MIT
