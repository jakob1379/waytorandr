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
- Set a preferred default profile for each display fingerprint
- Automatically restore the best layout when outputs change
- Emit machine-readable JSON from most commands with `--json`
- Offer dynamic shell completion for current-setup saved profile names and `set` targets

## Backend Support

`waytorandr` chooses a backend at runtime based on the active session.

- `wlroots` compositors
- KDE Plasma via KScreen
- GNOME via Mutter DisplayConfig

Actively exercised on Niri and KDE Plasma/KWin.

Set `WAYTORANDR_BACKEND=wlroots`, `WAYTORANDR_BACKEND=kscreen`, or
`WAYTORANDR_BACKEND=gnome` to force a backend in nested or mixed sessions where
desktop environment variables point at the wrong display stack.

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

The Flatpak and Snap artifacts are compatibility packages, not strong sandboxes.
Flatpak needs Wayland/session-bus access plus access to the waytorandr XDG config
and state paths. The Snap is classic confined for the same compositor/session
integration reason.

Inspect the current layout and save a profile:

```bash
./result/bin/waytorandr status
./result/bin/waytorandr save work-dock --setup-name office
./result/bin/waytorandr set auto
./result/bin/waytorandr set work-dock
./result/bin/waytorandr set work-dock --default
./result/bin/waytorandr set vertical --save
./result/bin/waytorandr set vertical --default
```

Preview a virtual layout without applying it:

```bash
./result/bin/waytorandr set horizontal --dry-run
./result/bin/waytorandr set mirror --dry-run
```

## CLI

```text
waytorandr set       Set a saved target, virtual configuration, or `auto` selection
waytorandr save      Save the current compositor layout as a profile
waytorandr remove    Remove a saved profile for the current setup
waytorandr cycle     Set the next saved profile
waytorandr status    Show the current layout state and related saved profiles
waytorandr version   Show the waytorandr version
waytorandr service   Manage the waytorandrd user service
```

Built-in `set` targets:

- `auto` - recover blank internal-only topologies with the built-in fallback, then apply the setup default or best matching saved profile when the current outputs have strong live identity
- `off` - disable external outputs and keep built-in panels on when present
- `external` - prefer external outputs; if none are present, keep built-in panels enabled
- `common` - clone all connected outputs at the largest shared resolution
- `largest` - clone all connected outputs at the same origin using each output's largest mode
- `mirror` - native mirroring at one shared mode
- `horizontal`
- `vertical`

Bare `waytorandr set` is rejected; use `waytorandr set auto` for automatic
selection.

If a saved profile name collides with `auto` or a virtual set target, select it
explicitly with `waytorandr set --profile <name>`.

`set` refuses to apply a layout that would leave every real output disabled.
`save` refuses to persist a profile from a topology with no enabled real outputs.
Use `--force` only when you accept applying through a backend whose validation
result is unsupported; it does not override the blank-layout safety gate.

`--default` is setup-local: it only updates the default for the current setup
fingerprint. With virtual targets, `set --default` saves the resulting layout as
profile `default` and makes that saved profile the default for the current setup.

Run `waytorandr set --help`, `waytorandr save --help`, and `waytorandr status --help`
for the full command reference and examples.

> [!TIP]
> Use `--json` on supported commands when you want stable machine-readable output.
> Human-readable output uses color when stdout is a terminal, `TERM` is not `dumb`, and `NO_COLOR` is unset. Set `CLICOLOR_FORCE=1` to force color for non-terminal output.
> Human-readable output and daemon logs escape terminal control characters from profile, backend, and monitor strings. JSON preserves raw values through serde escaping.
> Set `WAYTORANDR_REDACT_MONITORS=1` to redact monitor identifiers from human topology and plan output.
> `waytorandr service run` does not support `--json`.
> `waytorandr remove --dry-run --json` reports `would_remove`; applied `remove --json` reports `removed`. Removing a missing profile exits non-zero, and JSON mode still emits the `removed: false` payload before returning the error.
> `waytorandr status --json` includes optional `setup_name` and `builtin_output` fields when they are configured.

## Daemon

`waytorandrd` watches for physical display changes such as dock, undock, and
hotplug events. It does not react to compositor-only layout changes on an
unchanged set of connected displays. If the current topology is blank and only
built-in/internal panels are connected, it applies an immutable built-in fallback
before user defaults or saved profiles. Otherwise, when the physical setup
changes, it tries the configured default profile for the current fingerprint,
then the best matching saved profile, then a remembered layout for that setup,
and finally falls back to remembering the current topology. Remembered layouts
that would leave all real outputs disabled are skipped instead of being reused.
Blank layouts are not remembered as setup state.
Automatic profile apply is skipped when the current topology only has weak,
connector-only output identity; cached monitor identity is not treated as live
trust for `set auto` or daemon apply.

By default, the daemon logs at `info` level and still honors `RUST_LOG` when no
explicit log-level option is passed. Use `waytorandrd --log-level debug` or the
shortcut `waytorandrd --verbose` to enable debug logging for foreground runs or
custom service units.

Profiles are trusted executable content when they define hooks. Hooks run as the
current user with `Command::new` and argv, not through a shell unless the profile
explicitly runs one. Hook stdout and stderr are discarded, hook timeouts are
clamped to 1-300 seconds, and timed-out hooks are killed as a process group on
Unix. Use `waytorandrd --no-hooks` or `waytorandr service run --no-hooks` to run
the daemon without executing profile hooks. Applying a hook-bearing profile emits
a warning because editing the profile file is equivalent to editing commands that
will execute as your user.

You can give a setup a stable friendly alias such as `office` or `meetingroom-01`
with `waytorandr save --setup-name <name>`. Matching still uses the raw setup
fingerprint internally; the alias is only for display and organization.

For non-Home-Manager setups, `waytorandr` can manage a user service directly:

```bash
waytorandr service install
waytorandr service start
waytorandr service status
```

`waytorandr service run` starts the daemon in the foreground and does not support `--json`. Add `--no-hooks` to disable daemon hook execution for that foreground run.
`waytorandr service install` writes an absolute path to the sibling `waytorandrd`
binary into the user unit; command discovery still depends on the `waytorandr`
binary you invoked.

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

The Home Manager unit uses an absolute `ExecStart` from the configured package.
Values under `services.waytorandr.environment` are trusted same-user input.

Profiles and saved default settings remain in standard XDG config, while runtime state remains in XDG state:

- `$XDG_CONFIG_HOME/waytorandr/waytorandr.json`
- `$XDG_STATE_HOME/waytorandr/state.toml`

Saved output layouts include the physical mode, scale factor, and may include a
derived `scaled_resolution` as `{ "width": ..., "height": ... }` only when it can
be computed (i.e., when derivable from mode, scale and transform). The value is
the logical footprint used for layout positioning: physical mode divided by scale,
with width and height swapped for rotated transforms. Serialization omits
`scaled_resolution` when derivation is unavailable (None). Manual edits are
recalculated from `mode`, `scale`, and `transform` during normalization of profiles
or observed topologies.

If the XDG variables are unset, these typically resolve to:

- `~/.config/waytorandr/waytorandr.json`
- `~/.local/state/waytorandr/state.toml`

These files are same-user trust boundaries. A local process that can edit them
can change display policy and, for hook-bearing profiles, commands executed by
`waytorandr` or `waytorandrd`. Oversized profile/state files and hostile backend
topology payloads are rejected before full parsing/planning.

Virtual and ignored outputs are preserved in layout data but excluded from setup
fingerprints and auto-apply identity decisions.

Shell completion does not query the live compositor; it lists saved profile names
from the profile store and may include profiles for other setups.

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
