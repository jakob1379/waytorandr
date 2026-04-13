{ self, pkgs, system, workspace }:

let
  lib = pkgs.lib;
  rust = pkgs.rust-bin.stable.latest.default;
  isPortableHost = system == "x86_64-linux";

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

  commonMeta = with lib; {
    description = workspace.description;
    homepage = workspace.homepage;
    license = licenses.mit;
    platforms = platforms.linux;
  };

  mkWaytorandrPackage =
    {
      pname ? "waytorandr",
      rustPlatform ? pkgs.rustPlatform,
      stdenv ? pkgs.stdenv,
      nativeBuildInputs ? packageNativeBuildInputs,
      buildInputs ? packageBuildInputs,
      cargoBuildTarget ? null,
    }:
    rustPlatform.buildRustPackage ({
      inherit pname stdenv nativeBuildInputs buildInputs;
      version = workspace.version;
      src = self;

      cargoLock.lockFile = self + /Cargo.lock;
      doCheck = false;

      meta = commonMeta;
    }
    // lib.optionalAttrs (cargoBuildTarget != null) {
      CARGO_BUILD_TARGET = cargoBuildTarget;
    });

  waytorandrPackage = mkWaytorandrPackage { };

  portableTarget =
    if isPortableHost
    then pkgs.pkgsCross.musl64.stdenv.hostPlatform.rust.rustcTarget
    else null;

  portablePackage =
    if !isPortableHost
    then null
    else mkWaytorandrPackage {
      pname = "waytorandr-portable";
      rustPlatform = pkgs.pkgsCross.musl64.rustPlatform;
      stdenv = pkgs.pkgsCross.musl64.stdenv;
      nativeBuildInputs = with pkgs; [ pkg-config clang lld ];
      buildInputs = [ ];
      cargoBuildTarget = portableTarget;
    };

  portableMuslRoot =
    if isPortableHost
    then pkgs.pkgsCross.musl64.musl
    else null;

  portableLibgccRoot =
    if isPortableHost
    then pkgs.pkgsCross.musl64.stdenv.cc.cc.libgcc
    else null;

  mkBundledPortableRoot =
    {
      name,
      interpreterPath,
      runtimeLibPath,
    }:
    pkgs.runCommand name {
      nativeBuildInputs = [ pkgs.patchelf ];
    } ''
      mkdir -p "$out/bin" "$out/lib/waytorandr"

      cp ${portablePackage}/bin/waytorandr "$out/bin/waytorandr"
      cp ${portablePackage}/bin/waytorandrd "$out/bin/waytorandrd"
      cp ${portableMuslRoot}/lib/libc.so "$out/lib/waytorandr/libc.so"
      cp ${portableMuslRoot}/lib/libc.so "$out/lib/waytorandr/ld-musl-x86_64.so.1"
      cp ${portableLibgccRoot}/x86_64-unknown-linux-musl/lib/libgcc_s.so.1 "$out/lib/waytorandr/libgcc_s.so.1"
      chmod 0755 "$out/bin/waytorandr" "$out/bin/waytorandrd" "$out/lib/waytorandr/libc.so" "$out/lib/waytorandr/ld-musl-x86_64.so.1"
      chmod 0644 "$out/lib/waytorandr/libgcc_s.so.1"

      patchelf --set-interpreter ${interpreterPath} --set-rpath ${runtimeLibPath} "$out/bin/waytorandr"
      patchelf --set-interpreter ${interpreterPath} --set-rpath ${runtimeLibPath} "$out/bin/waytorandrd"
    '';

  distroPortableRoot =
    if !isPortableHost
    then null
    else mkBundledPortableRoot {
      name = "waytorandr-distro-root-${workspace.version}";
      interpreterPath = "/usr/lib/waytorandr/ld-musl-x86_64.so.1";
      runtimeLibPath = "/usr/lib/waytorandr";
    };

  flatpakPortableRoot =
    if !isPortableHost
    then null
    else mkBundledPortableRoot {
      name = "waytorandr-flatpak-root-${workspace.version}";
      interpreterPath = "/app/lib/waytorandr/ld-musl-x86_64.so.1";
      runtimeLibPath = "/app/lib/waytorandr";
    };

  snapPortableRoot =
    if !isPortableHost
    then null
    else mkBundledPortableRoot {
      name = "waytorandr-snap-root-${workspace.version}";
      interpreterPath = "/snap/waytorandr/current/lib/waytorandr/ld-musl-x86_64.so.1";
      runtimeLibPath = "/snap/waytorandr/current/lib/waytorandr";
    };
in
{
  inherit
    devShellBuildInputs
    devShellTools
    distroPortableRoot
    flatpakPortableRoot
    isPortableHost
    lib
    runtimeLibraries
    rust
    snapPortableRoot
    portablePackage
    waytorandrPackage
    ;
}
