{pkgs}: let
  src = ../apps/noctalia;
  manifest = builtins.fromTOML (builtins.readFile (src + "/plugin.toml"));
in
  pkgs.stdenvNoCC.mkDerivation {
    pname = "waytorandr-noctalia-plugin";
    inherit (manifest) version;
    inherit src;
    dontBuild = true;

    nativeCheckInputs = [pkgs.luau];
    doCheck = true;
    checkPhase = ''
      runHook preCheck
      luau layouts.test.luau
      runHook postCheck
    '';

    installPhase = ''
      runHook preInstall
      dest=$out/share/noctalia/plugins/waytorandr
      mkdir -p $dest
      cp plugin.toml layouts.luau panel.luau widget.luau $dest/
      runHook postInstall
    '';
  }
