{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.waytorandr;
  package = self.packages.${pkgs.stdenv.hostPlatform.system}.waytorandr;
in
{
  options.services.waytorandr.enable = lib.mkEnableOption "waytorandr output profile daemon";

  config = lib.mkIf cfg.enable {
    home.packages = [ package ];

    systemd.user.services.waytorandr = {
      Unit = {
        Description = "waytorandr output profile daemon";
        After = [ "graphical-session-pre.target" ];
        PartOf = [ "graphical-session.target" ];
      };

      Service = {
        ExecStart = "${package}/bin/waytorandrd";
        Restart = "on-failure";
        RestartSec = 2;
      };

      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
