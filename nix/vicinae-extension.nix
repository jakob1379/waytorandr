{
  pkgs,
  workspace,
}: let
  src = ../apps/vicinae;
  inherit (pkgs.lib.importJSON (src + "/package.json")) name;
in
  pkgs.buildNpmPackage {
    inherit name src;
    inherit (workspace) version;
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
      cp -r "$HOME/.local/share/vicinae/extensions/${name}/." $out/
      runHook postInstall
    '';
  }
