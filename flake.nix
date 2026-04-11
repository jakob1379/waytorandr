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
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      workspaceVersion = cargoToml.workspace.package.version;
      workspaceTag = "v${workspaceVersion}";
      workspaceDescription = "Wayland-native display profile manager inspired by autorandr.";
      workspaceHomepage = cargoToml.workspace.package.repository or "https://github.com/jakob1379/waytorandr";
      workspaceSourceUrl = "${workspaceHomepage}/archive/refs/tags/${workspaceTag}.tar.gz";
      workspaceSourceTarballHash = "50907f3da9181d4aa1e8b76d5466cb865dcf06b99898ede36506ff489651cc53";
      homeModule = import ./nix/home-manager/waytorandr.nix { inherit self; };
    in
    utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs { inherit system overlays; };
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
          description = workspaceDescription;
          homepage = workspaceHomepage;
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
            version = workspaceVersion;
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
            name = "waytorandr-distro-root-${workspaceVersion}";
            interpreterPath = "/usr/lib/waytorandr/ld-musl-x86_64.so.1";
            runtimeLibPath = "/usr/lib/waytorandr";
          };
        flatpakPortableRoot =
          if !isPortableHost
          then null
          else mkBundledPortableRoot {
            name = "waytorandr-flatpak-root-${workspaceVersion}";
            interpreterPath = "/app/lib/waytorandr/ld-musl-x86_64.so.1";
            runtimeLibPath = "/app/lib/waytorandr";
          };
        snapPortableRoot =
          if !isPortableHost
          then null
          else mkBundledPortableRoot {
            name = "waytorandr-snap-root-${workspaceVersion}";
            interpreterPath = "/snap/waytorandr/current/lib/waytorandr/ld-musl-x86_64.so.1";
            runtimeLibPath = "/snap/waytorandr/current/lib/waytorandr";
          };
        mkNfpmPackage =
          {
            format,
            targetName,
          }:
          pkgs.runCommand "waytorandr-${format}-${workspaceVersion}" {
            nativeBuildInputs = [ pkgs.nfpm ];
          } ''
            mkdir -p "$out"
            cat > nfpm.yaml <<EOF
            name: waytorandr
            arch: amd64
            platform: linux
            version: ${workspaceVersion}
            release: "1"
            section: utils
            priority: optional
            maintainer: "waytorandr contributors"
            vendor: "waytorandr contributors"
            description: |
              ${workspaceDescription}
            homepage: "${workspaceHomepage}"
            license: "MIT"
            contents:
              - src: ${distroPortableRoot}/bin/waytorandr
                dst: /usr/bin/waytorandr
                file_info:
                  mode: 0755
              - src: ${distroPortableRoot}/bin/waytorandrd
                dst: /usr/bin/waytorandrd
                file_info:
                  mode: 0755
              - src: ${distroPortableRoot}/lib/waytorandr/ld-musl-x86_64.so.1
                dst: /usr/lib/waytorandr/ld-musl-x86_64.so.1
                file_info:
                  mode: 0755
              - src: ${distroPortableRoot}/lib/waytorandr/libc.so
                dst: /usr/lib/waytorandr/libc.so
                file_info:
                  mode: 0755
              - src: ${distroPortableRoot}/lib/waytorandr/libgcc_s.so.1
                dst: /usr/lib/waytorandr/libgcc_s.so.1
                file_info:
                  mode: 0644
              - src: ${./README.md}
                dst: /usr/share/doc/waytorandr/README.md
                file_info:
                  mode: 0644
              - src: ${./LICENSE}
                dst: /usr/share/licenses/waytorandr/LICENSE
                file_info:
                  mode: 0644
            EOF
            nfpm package --config nfpm.yaml --packager ${format} --target "$out/${targetName}"
          '';
        autPackage =
          if !isPortableHost
          then null
          else mkNfpmPackage {
            format = "archlinux";
            targetName = "waytorandr-${workspaceVersion}-1-x86_64.pkg.tar.zst";
          };
        apkPackage =
          if !isPortableHost
          then null
          else mkNfpmPackage {
            format = "apk";
            targetName = "waytorandr-${workspaceVersion}-r1.apk";
          };
        debPackage =
          if !isPortableHost
          then null
          else mkNfpmPackage {
            format = "deb";
            targetName = "waytorandr_${workspaceVersion}_amd64.deb";
          };
        rpmPackage =
          if !isPortableHost
          then null
          else mkNfpmPackage {
            format = "rpm";
            targetName = "waytorandr-${workspaceVersion}-1.x86_64.rpm";
          };
        aurPackage =
          if !isPortableHost
          then null
          else pkgs.runCommand "waytorandr-aur-${workspaceVersion}" { } ''
            mkdir -p "$out"
            cat > "$out/PKGBUILD" <<EOF
            # Maintainer: waytorandr contributors
            pkgname=waytorandr
            pkgver=${workspaceVersion}
            pkgrel=1
            pkgdesc='${workspaceDescription}'
            arch=('x86_64')
            url='${workspaceHomepage}'
            license=('MIT')
            depends=('gcc-libs' 'glibc' 'libxkbcommon' 'wayland' 'wlroots0.18')
            makedepends=('rust' 'clang' 'lld' 'pkgconf' 'wayland-protocols')
            source=('${workspaceSourceUrl}')
            sha256sums=('${workspaceSourceTarballHash}')

            build() {
              cd "\$srcdir/waytorandr-${workspaceVersion}"
              cargo build --release
            }

            package() {
              cd "\$srcdir/waytorandr-${workspaceVersion}"
              install -Dm755 target/release/waytorandr "\$pkgdir/usr/bin/waytorandr"
              install -Dm755 target/release/waytorandrd "\$pkgdir/usr/bin/waytorandrd"
              install -Dm644 README.md "\$pkgdir/usr/share/doc/waytorandr/README.md"
              install -Dm644 LICENSE "\$pkgdir/usr/share/licenses/waytorandr/LICENSE"
            }
            EOF

            cat > "$out/.SRCINFO" <<EOF
            pkgbase = waytorandr
            	pkgdesc = ${workspaceDescription}
            	pkgver = ${workspaceVersion}
            	pkgrel = 1
            	url = ${workspaceHomepage}
            	arch = x86_64
            	license = MIT
            	makedepends = rust
            	makedepends = clang
            	makedepends = lld
            	makedepends = pkgconf
            	makedepends = wayland-protocols
            	depends = gcc-libs
            	depends = glibc
            	depends = libxkbcommon
            	depends = wayland
            	depends = wlroots0.18
            	source = ${workspaceSourceUrl}
            	sha256sums = ${workspaceSourceTarballHash}

            pkgname = waytorandr
            EOF
          '';
        flatpakPackage =
          if !isPortableHost
          then null
          else pkgs.runCommand "waytorandr-flatpak-${workspaceVersion}" {
            nativeBuildInputs = with pkgs; [ flatpak ostree squashfsTools ];
          } ''
            app_id="io.github.jakob1379.waytorandr"
            build_dir="$TMPDIR/build"
            repo_dir="$TMPDIR/repo"
            mkdir -p "$build_dir/files/bin" "$build_dir/files/lib/waytorandr" "$build_dir/files/share/doc/waytorandr" "$build_dir/var/tmp" "$repo_dir"

            cp ${flatpakPortableRoot}/bin/waytorandr "$build_dir/files/bin/waytorandr"
            cp ${flatpakPortableRoot}/bin/waytorandrd "$build_dir/files/bin/waytorandrd"
            cp ${flatpakPortableRoot}/lib/waytorandr/ld-musl-x86_64.so.1 "$build_dir/files/lib/waytorandr/ld-musl-x86_64.so.1"
            cp ${flatpakPortableRoot}/lib/waytorandr/libc.so "$build_dir/files/lib/waytorandr/libc.so"
            cp ${flatpakPortableRoot}/lib/waytorandr/libgcc_s.so.1 "$build_dir/files/lib/waytorandr/libgcc_s.so.1"
            cp ${./README.md} "$build_dir/files/share/doc/waytorandr/README.md"
            cp ${./LICENSE} "$build_dir/files/share/doc/waytorandr/LICENSE"
            chmod 0755 "$build_dir/files/bin/waytorandr" "$build_dir/files/bin/waytorandrd" "$build_dir/files/lib/waytorandr/ld-musl-x86_64.so.1" "$build_dir/files/lib/waytorandr/libc.so"
            chmod 0644 "$build_dir/files/lib/waytorandr/libgcc_s.so.1"
            chmod 0644 "$build_dir/files/share/doc/waytorandr/README.md" "$build_dir/files/share/doc/waytorandr/LICENSE"

            cat > "$build_dir/metadata" <<EOF
            [Application]
            name=$app_id
            runtime=org.freedesktop.Platform/x86_64/24.08
            sdk=org.freedesktop.Sdk/x86_64/24.08
            command=waytorandr
            EOF

            flatpak build-finish \
              --command=waytorandr \
              --socket=wayland \
              --socket=session-bus \
              --share=ipc \
              --share=network \
              --filesystem=home \
              "$build_dir"

            ostree init --repo="$repo_dir" --mode=archive-z2
            flatpak build-export --disable-sandbox --arch=x86_64 "$repo_dir" "$build_dir" stable

            mkdir -p "$out"
            flatpak build-bundle \
              --arch=x86_64 \
              --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
              "$repo_dir" \
              "$out/waytorandr-${workspaceVersion}.flatpak" \
              "$app_id" \
              stable
          '';
        snapPackage =
          if !isPortableHost
          then null
          else pkgs.runCommand "waytorandr-snap-${workspaceVersion}" {
            nativeBuildInputs = with pkgs; [ squashfsTools ];
          } ''
            snap_root="$TMPDIR/snap"
            mkdir -p "$snap_root/bin" "$snap_root/lib/waytorandr" "$snap_root/meta" "$out"

            cp ${snapPortableRoot}/bin/waytorandr "$snap_root/bin/waytorandr"
            cp ${snapPortableRoot}/bin/waytorandrd "$snap_root/bin/waytorandrd"
            cp ${snapPortableRoot}/lib/waytorandr/ld-musl-x86_64.so.1 "$snap_root/lib/waytorandr/ld-musl-x86_64.so.1"
            cp ${snapPortableRoot}/lib/waytorandr/libc.so "$snap_root/lib/waytorandr/libc.so"
            cp ${snapPortableRoot}/lib/waytorandr/libgcc_s.so.1 "$snap_root/lib/waytorandr/libgcc_s.so.1"
            chmod 0755 "$snap_root/bin/waytorandr" "$snap_root/bin/waytorandrd" "$snap_root/lib/waytorandr/ld-musl-x86_64.so.1" "$snap_root/lib/waytorandr/libc.so"
            chmod 0644 "$snap_root/lib/waytorandr/libgcc_s.so.1"

            cat > "$snap_root/meta/snap.yaml" <<EOF
            name: waytorandr
            version: "${workspaceVersion}"
            summary: Wayland-native display profile manager
            description: |
              ${workspaceDescription}
            grade: stable
            confinement: classic
            base: core24
            apps:
              waytorandr:
                command: bin/waytorandr
              waytorandrd:
                command: bin/waytorandrd
            EOF

            mksquashfs "$snap_root" "$out/waytorandr_${workspaceVersion}_amd64.snap" -all-root -noappend -quiet
          '';
        linuxPortablePackages = lib.optionalAttrs isPortableHost {
          portable = portablePackage;
          aut = autPackage;
          archlinux = autPackage;
          apk = apkPackage;
          deb = debPackage;
          rpm = rpmPackage;
          aur = aurPackage;
          pkgbuild = aurPackage;
          flatpak = flatpakPackage;
          snap = snapPackage;
        };
      in
      {
        devShell = pkgs.mkShell {
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
        };

        packages = {
          waytorandr = waytorandrPackage;
          default = waytorandrPackage;
        } // linuxPortablePackages;
      }
    )
    // {
      homeModules = {
        waytorandr = homeModule;
        default = homeModule;
      };
      homeManagerModules = self.homeModules;
    };
}
