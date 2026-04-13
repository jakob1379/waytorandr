{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    utils,
    rust-overlay,
  }: let
    workspace = import ./nix/workspace.nix {inherit self;};
    homeModule = import ./nix/home-manager/waytorandr.nix {inherit self;};
  in
    utils.lib.eachDefaultSystem (
      system: let
        perSystem = import ./nix/per-system.nix {
          inherit self nixpkgs rust-overlay system workspace;
        };
      in
        perSystem
    )
    // {
      homeModules = {
        waytorandr = homeModule;
        default = homeModule;
      };
      homeManagerModules = self.homeModules;
    };
}
