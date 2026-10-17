{
  pkgs,
  workspace,
}: let
  src = ../apps/vicinae;
in
  pkgs.buildNpmPackage {
    pname = "waytorandr-vicinae-extension";
    inherit (workspace) version;
    inherit src;
    npmDeps = pkgs.importNpmLock {npmRoot = src;};
    inherit (pkgs.importNpmLock) npmConfigHook;

    doCheck = true;
    checkPhase = ''
      runHook preCheck
      npm test
      npm run typecheck
      runHook postCheck
    '';

    installPhase = ''
      runHook preInstall
      mkdir -p $out
      cp -r "$HOME/.local/share/vicinae/extensions/waytorandr/." $out/
      runHook postInstall
    '';
  }
