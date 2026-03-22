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
    utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs { inherit system overlays; };
        rust = pkgs.rust-bin.stable.latest.default;
        wayland = pkgs.wayland;
      in
      {
        devShell = pkgs.mkShell {
          inherit rust;

          buildInputs = with pkgs; [
            rust
            cargo
            rustfmt
            clippy
            pkg-config
            wayland
            wayland-protocols
            wlroots
            libxkbcommon.dev
            systemd.dev
            libdrm.dev
          ];

          LIBCLUDIR = "${pkgs.libglvnd}/lib";

          shellHook = ''
            export WAYLAND_DISPLAY=${pkgs.xorg.libX11}/lib
            export LD_LIBRARY_PATH="${pkgs.wayland}/lib:$LD_LIBRARY_PATH"
            '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "waytorandr";
          version = "0.1.0";
          src = self;

          cargoLock.lockFile = self + /Cargo.lock;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [
            wayland
            wayland-protocols
            wlroots
            libxkbcommon.dev
          ];

          meta = with pkgs.lib; {
            description = "Wayland-native display profile manager";
            homepage = "https://github.com/jsg/waytorandr";
            license = licenses.mit;
            platforms = platforms.linux;
          };
        };
      }
    );
}
