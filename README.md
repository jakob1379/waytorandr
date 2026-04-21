# waytorandr

`waytorandr` is a Wayland-native display profile manager inspired by `autorandr`.
It saves the current layout as a named profile, reapplies profiles on demand,
and can automatically restore the right layout when outputs change.

It includes:

- `waytorandr`: CLI for saving, applying, and inspecting display profiles
- `waytorandrd`: daemon for automatic re-application after dock, undock, or hotplug events

## Scope

- Save the current compositor layout as a named profile
- Optionally assign a friendly name to each detected display setup
- Reapply a saved profile or a built-in virtual layout
- Set a preferred default profile for each display fingerprint and a default target for new setups
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
nix build .#flatpak
nix build .#snap
```

Inspect the current layout and save a profile:

```bash
./result/bin/waytorandr status
./result/bin/waytorandr save work-dock --setup-name office
./result/bin/waytorandr set work-dock
./result/bin/waytorandr set work-dock --default
./result/bin/waytorandr set external --default
./result/bin/waytorandr set vertical --default
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
waytorandr status    Show the current layout state and related saved profiles
waytorandr version   Show the waytorandr version
waytorandr service   Manage the waytorandrd user service
```

Built-in `set` targets:

- `off` - disable external outputs and keep built-in panels on when present
- `external` - prefer external outputs; if none are present, keep built-in panels enabled
- `common` - clone all connected outputs at the largest shared resolution
- `largest` - clone all connected outputs at the same origin using each output's largest mode
- `mirror` - native mirroring at one shared mode
- `horizontal`
- `vertical`

Run `waytorandr set --help`, `waytorandr save --help`, and `waytorandr status --help`
for the full command reference and examples.

> [!TIP]
> Use `--json` on supported commands when you want stable machine-readable output.
> Human-readable output uses color when stdout is a terminal, `TERM` is not `dumb`, and `NO_COLOR` is unset. Set `CLICOLOR_FORCE=1` to force color for non-terminal output.
> `waytorandr service run` does not support `--json`.
> `waytorandr remove --dry-run --json` reports `would_remove`; applied `remove --json` reports `removed`.
> `waytorandr status --json` includes optional `setup_name`, `builtin_output`, and `new_setup_default` fields when they are configured.

## Daemon

`waytorandrd` watches for physical display changes such as dock, undock, and
hotplug events. It does not react to compositor-only layout changes on an
unchanged set of connected displays. When the physical setup changes, it first
tries the configured default profile for the current fingerprint, then the best
matching saved profile, then a remembered layout for that setup, then
the configured default target for new setups, and finally falls back to
remembering the current topology. Remembered layouts that would leave all real
outputs disabled are skipped instead of being reused.

If you want the fallback target for new setups to mean "internal panel only",
configure `builtin` in `waytorandr.json`. This is a config-only fallback target;
it is not advertised as an explicit `waytorandr set` target.

```json
{
  "settings": {
    "builtin_output": {
      "connector": "eDP-1"
    },
    "new_setup_default": {
      "kind": "virtual",
      "preset": "builtin"
    }
  }
}
```

`builtin_output` is optional. When omitted, waytorandr falls back to its built-in
display heuristics. When set, it overrides those heuristics and treats the matching
output identity as the built-in display for `builtin` fallback decisions.

To avoid reinforcing transient compositor glitches during output churn, the
daemon ignores intermediate topologies that leave every real output disabled
and does not remember those blank layouts as setup state.

You can give a setup a stable friendly alias such as `office` or `meetingroom-01`
with `waytorandr save --setup-name <name>`. Matching still uses the raw setup
fingerprint internally; the alias is only for display and organization.

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

Profiles and saved default settings remain in standard XDG config, while runtime state remains in XDG state:

- `$XDG_CONFIG_HOME/waytorandr/waytorandr.json`
- `$XDG_STATE_HOME/waytorandr/state.toml`

If the XDG variables are unset, these typically resolve to:

- `~/.config/waytorandr/waytorandr.json`
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
nix develop -c prek install
nix develop -c prek run --all-files
nix develop -c just check
```

More development details live in `DEVELOPMENT.md`.

## License

MIT
