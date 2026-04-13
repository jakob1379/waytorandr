{
  pkgs,
  rust,
  runtimeLibraries,
  devShellTools,
  devShellBuildInputs,
}:

pkgs.mkShell {
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
}
