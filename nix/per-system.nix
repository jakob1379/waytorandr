{
  self,
  git-hooks,
  nixpkgs,
  rust-overlay,
  system,
  workspace,
}: let
  overlays = [rust-overlay.overlays.default];
  pkgs = import nixpkgs {inherit system overlays;};
  context = import ./package-context.nix {
    inherit
      self
      pkgs
      system
      workspace
      ;
  };
  preCommitCheck = import ./git-hooks.nix {
    inherit git-hooks pkgs;
    src = self;
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
    extraPackages = preCommitCheck.enabledPackages;
    extraShellHook = preCommitCheck.shellHook;
  };
  inherit (preCommitCheck.config) package configFile;
  formatter = pkgs.writeShellScriptBin "pre-commit-run" ''
    ${pkgs.lib.getExe package} run --all-files --config ${configFile}
  '';
in {
  inherit devShell formatter;
  packages =
    packages
    // {
      default = packages.waytorandr;
    };
}
