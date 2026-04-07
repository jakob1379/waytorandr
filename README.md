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

Implementation files:

- `nix/home-manager/waytorandr.nix`
- `nix/modules/home-manager.nix`

Read `nix/modules/home-manager.nix` for the option surface and inline option
descriptions.

The module only installs the package and manages the `waytorandrd` user
service. Profiles remain imperative and are expected to be managed by
`waytorandr` itself.

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
          services.waytorandr.enable = true;
        }
      ];
    };
  };
}
```

Dynamic shell completion is built in. After enabling it for your shell, `waytorandr set <TAB>` and `waytorandr remove <TAB>` include saved profile names.

## Project Note

This project has been heavily AI-assisted.

I do not claim expert Rust knowledge, so treat the code as pragmatic and evolving rather than polished Rust craftsmanship.

## License

MIT OR Apache-2.0
