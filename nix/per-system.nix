{
  self,
  nixpkgs,
  rust-overlay,
  system,
  workspace,
}: let
  overlays = [rust-overlay.overlays.default];
  pkgs = import nixpkgs {inherit system overlays;};
  context = import ./package-context.nix {
    inherit self pkgs system workspace;
  };
  packages = import ./release-packages.nix {
    inherit pkgs workspace;
    inherit
      (context)
      distroPortableRoot
      flatpakPortableRoot
      isPortableHost
      lib
      portablePackage
      snapPortableRoot
      waytorandrPackage
      ;
  };
  devShell = import ./dev-shell.nix {
    inherit pkgs;
    inherit
      (context)
      devShellBuildInputs
      devShellTools
      runtimeLibraries
      rust
      ;
  };
in {
  inherit devShell;
  formatter = pkgs.alejandra;
  packages = packages // {default = packages.waytorandr;};
}
