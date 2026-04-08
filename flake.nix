{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, utils, rust-overlay }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      workspaceVersion = cargoToml.workspace.package.version;
      homeModule = import ./nix/home-manager/waytorandr.nix { inherit self; };
    in
    utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs { inherit system overlays; };
        rust = pkgs.rust-bin.stable.latest.default;
        devShellTools = with pkgs; [
          rust
          cargo
          rustfmt
          clippy
          clang
          hyperfine
          lld
          sccache
          pkg-config
        ];
        runtimeLibraries = with pkgs; [
          wayland
        ];
        packageNativeBuildInputs = with pkgs; [ pkg-config clang lld ];
        packageBuildInputs = with pkgs; [
          wayland-protocols
          wlroots
          libxkbcommon.dev
        ] ++ runtimeLibraries;
        devShellBuildInputs = with pkgs; [
          systemd.dev
          libdrm.dev
        ] ++ packageBuildInputs;
        waytorandrPackage = pkgs.rustPlatform.buildRustPackage {
          pname = "waytorandr";
          version = workspaceVersion;
          src = self;

          cargoLock.lockFile = self + /Cargo.lock;

          nativeBuildInputs = packageNativeBuildInputs;
          buildInputs = packageBuildInputs;

          meta = with pkgs.lib; {
            description = "Wayland-native display profile manager";
            homepage = "https://github.com/jsg/waytorandr";
            license = licenses.mit;
            platforms = platforms.linux;
          };
        };
      in
      {
        devShell = pkgs.mkShell {
          inherit rust;

          packages = devShellTools;
          buildInputs = devShellBuildInputs;

          LIBCLUDIR = "${pkgs.libglvnd}/lib";

          shellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibraries}:$LD_LIBRARY_PATH"
            export SCCACHE_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/sccache"
            export SCCACHE_BASEDIR="$PWD"
            if [ -z "$GITHUB_ACTIONS" ]; then
              export RUSTC_WRAPPER="${pkgs.sccache}/bin/sccache"
            fi
            '';
        };

        packages = {
          waytorandr = waytorandrPackage;
          default = waytorandrPackage;
        };
      }
    )
    // {
      homeModules = {
        waytorandr = homeModule;
        default = homeModule;
      };
      homeManagerModules = self.homeModules;
    };
}
