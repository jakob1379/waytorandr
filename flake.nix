{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    git-hooks.url = "github:cachix/git-hooks.nix";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    git-hooks,
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
          inherit self git-hooks nixpkgs rust-overlay system workspace;
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
