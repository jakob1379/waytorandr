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
      description = ''
        Package providing the `waytorandr` and `waytorandrd` binaries.

        Enabling the module installs this package into `home.packages`. Profile
        data still lives in the application's XDG config/state files rather than
        in Home Manager options.
      '';
    };

    environment = mkOption {
      type = with types; attrsOf str;
      default = { };
      description = ''
        Environment variables passed to the `waytorandrd` user service.

        This is for service runtime configuration only. Saved profiles and
        defaults are still managed through the CLI and persisted under
        `$XDG_CONFIG_HOME/waytorandr` and `$XDG_STATE_HOME/waytorandr`.
      '';
      example = {
        RUST_LOG = "waytorandrd=debug";
      };
    };

    systemdTarget = mkOption {
      type = types.str;
      default = config.wayland.systemd.target;
      defaultText = literalExpression "config.wayland.systemd.target";
      description = ''
        Systemd user target that should own the `waytorandrd` service.

        The module assumes a working graphical Wayland session. By default this
        follows `config.wayland.systemd.target`, and the service also requires
        `WAYLAND_DISPLAY` to be present before it starts.
      '';
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
