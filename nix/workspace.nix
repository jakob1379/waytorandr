{self}: let
  cargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);
  inherit (cargoToml.workspace.package) version;
  tag = "v${version}";
  description = "Wayland-native display profile manager inspired by autorandr.";
  homepage = cargoToml.workspace.package.repository or "https://github.com/jakob1379/waytorandr";
  revision = self.rev or null;
  aurSource =
    if revision != null
    then "waytorandr::git+${homepage}.git#commit=${revision}"
    else "waytorandr::git+${homepage}.git#tag=${tag}";
in {
  inherit
    aurSource
    cargoToml
    description
    homepage
    revision
    tag
    version
    ;
}
