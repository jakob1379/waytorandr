self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    getExe'
    literalExpression
    mkEnableOption
    mkIf
    mkOption
    types
    ;

  cfg = config.services.waytorandr;
  defaultPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.waytorandr;
in
{
  options.services.waytorandr = {
    enable = mkEnableOption "waytorandr and its automatic waytorandrd daemon";

    package = mkOption {
      type = types.package;
      default = defaultPackage;
      defaultText = literalExpression "inputs.waytorandr.packages.${pkgs.system}.waytorandr";
      description = "Package providing the waytorandr and waytorandrd binaries.";
    };

    environment = mkOption {
      type = with types; attrsOf str;
      default = { };
      description = "Environment variables passed to the waytorandrd user service.";
      example = {
        RUST_LOG = "waytorandrd=debug";
      };
    };

    systemdTarget = mkOption {
      type = types.str;
      default = config.wayland.systemd.target;
      defaultText = literalExpression "config.wayland.systemd.target";
      description = "Systemd target that should own the waytorandrd user service.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      (lib.hm.assertions.assertPlatform "services.waytorandr" pkgs lib.platforms.linux)
    ];

    home.packages = [ cfg.package ];

    systemd.user.services.waytorandrd = {
      Unit = {
        Description = "Wayland display profile daemon";
        Documentation = "https://github.com/jsg/waytorandr";
        ConditionEnvironment = "WAYLAND_DISPLAY";
        PartOf = [ cfg.systemdTarget ];
        Requires = [ cfg.systemdTarget ];
        After = [ cfg.systemdTarget ];
      };

      Service = {
        Type = "simple";
        ExecStart = "${getExe' cfg.package "waytorandrd"}";
        Environment = lib.mapAttrsToList (name: value: "${name}=${value}") cfg.environment;
        Restart = "always";
        Slice = "background.slice";
      };

      Install = {
        WantedBy = [ cfg.systemdTarget ];
      };
    };
  };
}
