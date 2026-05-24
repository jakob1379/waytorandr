{
  pkgs,
  workspace,
  lib,
  isPortableHost,
  waytorandrPackage,
  portablePackage,
  distroPortableRoot,
  flatpakPortableRoot,
  snapPortableRoot,
}: let
  mkNfpmPackage = {
    format,
    targetName,
  }:
    pkgs.runCommand "waytorandr-${format}-${workspace.version}"
    {
      nativeBuildInputs = [pkgs.nfpm];
    }
    ''
      mkdir -p "$out"
      cat > nfpm.yaml <<EOF
      name: waytorandr
      arch: amd64
      platform: linux
      version: ${workspace.version}
      release: "1"
      section: utils
      priority: optional
      maintainer: "waytorandr contributors"
      vendor: "waytorandr contributors"
      description: |
        ${workspace.description}
      homepage: "${workspace.homepage}"
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
        - src: ${../README.md}
          dst: /usr/share/doc/waytorandr/README.md
          file_info:
            mode: 0644
        - src: ${../LICENSE}
          dst: /usr/share/licenses/waytorandr/LICENSE
          file_info:
            mode: 0644
      EOF
      nfpm package --config nfpm.yaml --packager ${format} --target "$out/${targetName}"
    '';

  autPackage =
    if !isPortableHost
    then null
    else
      mkNfpmPackage {
        format = "archlinux";
        targetName = "waytorandr-${workspace.version}-1-x86_64.pkg.tar.zst";
      };

  apkPackage =
    if !isPortableHost
    then null
    else
      mkNfpmPackage {
        format = "apk";
        targetName = "waytorandr-${workspace.version}-r1.apk";
      };

  debPackage =
    if !isPortableHost
    then null
    else
      mkNfpmPackage {
        format = "deb";
        targetName = "waytorandr_${workspace.version}_amd64.deb";
      };

  rpmPackage =
    if !isPortableHost
    then null
    else
      mkNfpmPackage {
        format = "rpm";
        targetName = "waytorandr-${workspace.version}-1.x86_64.rpm";
      };

  aurPackage =
    if !isPortableHost
    then null
    else
      pkgs.runCommand "waytorandr-aur-${workspace.version}" {} ''
        mkdir -p "$out"
        cat > "$out/PKGBUILD" <<EOF
        # Maintainer: waytorandr contributors
        pkgname=waytorandr
        pkgver=${workspace.version}
        pkgrel=1
        pkgdesc='${workspace.description}'
        arch=('x86_64')
        url='${workspace.homepage}'
        license=('MIT')
        depends=('gcc-libs' 'glibc' 'wayland')
        makedepends=('rust' 'pkgconf')
        source=('${workspace.aurSource}')
        sha256sums=('SKIP')

        build() {
          cd "\$srcdir/waytorandr"
          cargo build --release
        }

        package() {
          cd "\$srcdir/waytorandr"
          install -Dm755 target/release/waytorandr "\$pkgdir/usr/bin/waytorandr"
          install -Dm755 target/release/waytorandrd "\$pkgdir/usr/bin/waytorandrd"
          install -Dm644 README.md "\$pkgdir/usr/share/doc/waytorandr/README.md"
          install -Dm644 LICENSE "\$pkgdir/usr/share/licenses/waytorandr/LICENSE"
        }
        EOF

        cat > "$out/.SRCINFO" <<EOF
        pkgbase = waytorandr
        	pkgdesc = ${workspace.description}
        	pkgver = ${workspace.version}
        	pkgrel = 1
        	url = ${workspace.homepage}
        	arch = x86_64
        	license = MIT
        	makedepends = rust
        	makedepends = pkgconf
        	depends = gcc-libs
        	depends = glibc
        	depends = wayland
        	source = ${workspace.aurSource}
        	sha256sums = SKIP

        pkgname = waytorandr
        EOF
      '';

  flatpakPackage =
    if !isPortableHost
    then null
    else
      pkgs.runCommand "waytorandr-flatpak-${workspace.version}"
      {
        nativeBuildInputs = with pkgs; [
          flatpak
          ostree
          squashfsTools
        ];
      }
      ''
        app_id="io.github.jakob1379.waytorandr"
        build_dir="$TMPDIR/build"
        repo_dir="$TMPDIR/repo"
        mkdir -p "$build_dir/files/bin" "$build_dir/files/lib/waytorandr" "$build_dir/files/share/doc/waytorandr" "$build_dir/var/tmp" "$repo_dir"

        cp ${flatpakPortableRoot}/bin/waytorandr "$build_dir/files/bin/waytorandr"
        cp ${flatpakPortableRoot}/bin/waytorandrd "$build_dir/files/bin/waytorandrd"
        cp ${flatpakPortableRoot}/lib/waytorandr/ld-musl-x86_64.so.1 "$build_dir/files/lib/waytorandr/ld-musl-x86_64.so.1"
        cp ${flatpakPortableRoot}/lib/waytorandr/libc.so "$build_dir/files/lib/waytorandr/libc.so"
        cp ${flatpakPortableRoot}/lib/waytorandr/libgcc_s.so.1 "$build_dir/files/lib/waytorandr/libgcc_s.so.1"
        cp ${../README.md} "$build_dir/files/share/doc/waytorandr/README.md"
        cp ${../LICENSE} "$build_dir/files/share/doc/waytorandr/LICENSE"
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
          --filesystem=xdg-config/waytorandr:create \
          --filesystem=~/.local/state/waytorandr:create \
          "$build_dir"

        ostree init --repo="$repo_dir" --mode=archive-z2
        flatpak build-export --disable-sandbox --arch=x86_64 "$repo_dir" "$build_dir" stable

        mkdir -p "$out"
        flatpak build-bundle \
          --arch=x86_64 \
          --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
          "$repo_dir" \
          "$out/waytorandr-${workspace.version}.flatpak" \
          "$app_id" \
          stable
      '';

  snapPackage =
    if !isPortableHost
    then null
    else
      pkgs.runCommand "waytorandr-snap-${workspace.version}"
      {
        nativeBuildInputs = with pkgs; [squashfsTools];
      }
      ''
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
        version: "${workspace.version}"
        summary: Wayland-native display profile manager
        description: |
          ${workspace.description}
        grade: stable
        confinement: classic
        base: core24
        apps:
          waytorandr:
            command: bin/waytorandr
          waytorandrd:
            command: bin/waytorandrd
        EOF

        mksquashfs "$snap_root" "$out/waytorandr_${workspace.version}_amd64.snap" -all-root -noappend -quiet
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
    waytorandr = waytorandrPackage;
  }
  // linuxPortablePackages
