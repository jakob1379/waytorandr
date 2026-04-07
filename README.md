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
- the daemon uses the same wlroots backend path as the CLI and writes runtime state under the same XDG state directory

## Status

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

## Nix

```bash
nix build
nix develop
nix develop -c cargo test
./result/bin/waytorandr --help
./result/bin/waytorandrd
```

### Home Manager

The flake exports a Home Manager module at `homeManagerModules.default` and
`homeManagerModules.waytorandr`.

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    waytorandr.url = "github:jsg/waytorandr";
  };

  outputs = inputs@{ nixpkgs, home-manager, waytorandr, ... }: {
    homeConfigurations."alice" = home-manager.lib.homeManagerConfiguration {
      pkgs = import nixpkgs { system = "x86_64-linux"; };
      modules = [
        waytorandr.homeManagerModules.default
        {
          home.username = "alice";
          home.homeDirectory = "/home/alice";
          home.stateVersion = "24.11";

          services.waytorandr = {
            enable = true;
            environment.RUST_LOG = "waytorandrd=info";
            profiles = [
              {
                name = "laptop";
                layout = {
                  "eDP-1" = {
                    enabled = true;
                    scale = 2.0;
                    mode = {
                      width = 2880;
                      height = 1800;
                      refresh = 60000;
                    };
                  };
                };
              }
            ];
          };
        }
      ];
    };
  };
}
```

The module follows the same basic shape as Home Manager services like
`services.kanshi` and `services.swayidle`:

- `enable` installs the package and starts `waytorandrd` as a user service
- `profiles` declaratively writes `~/.config/waytorandr/profiles.json`
- `profilesFile` lets you provide an existing `profiles.json` instead
- `systemdTarget` binds the daemon to your graphical Wayland session target
- `environment` lets you pass service-specific variables such as `RUST_LOG`

The daemon still owns runtime state under `XDG_STATE_HOME/waytorandr`. In
particular, per-setup defaults remain mutable state, so the module does not
symlink `state.toml`. Set those defaults with the CLI after activation, for
example `waytorandr save desk --default` or `waytorandr set desk --default`.

Dynamic shell completion is built in. After enabling it for your shell, `waytorandr set <TAB>` and `waytorandr remove <TAB>` include saved profile names.

## Project Note

This project has been heavily AI-assisted.

I do not claim expert Rust knowledge, so treat the code as pragmatic and evolving rather than polished Rust craftsmanship.

## License

MIT OR Apache-2.0
