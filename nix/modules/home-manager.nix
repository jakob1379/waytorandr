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
  defaultPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
  jsonFormat = pkgs.formats.json { };

  identityModule = types.submodule {
    options = {
      edidHash = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "EDID hash used to match a physical display.";
      };

      make = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Display manufacturer name.";
      };

      model = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Display model name.";
      };

      serial = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Display serial number.";
      };

      connector = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Connector name such as `eDP-1` or `DP-2`.";
      };

      description = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Compositor-provided display description.";
      };

      isVirtual = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this output should be treated as virtual.";
      };

      isIgnored = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this output should be ignored when matching setups.";
      };
    };
  };

  positionModule = types.submodule {
    options = {
      x = mkOption {
        type = types.int;
        default = 0;
        description = "Horizontal position in compositor coordinates.";
      };

      y = mkOption {
        type = types.int;
        default = 0;
        description = "Vertical position in compositor coordinates.";
      };
    };
  };

  modeModule = types.submodule {
    options = {
      width = mkOption {
        type = types.ints.positive;
        description = "Output width in pixels.";
      };

      height = mkOption {
        type = types.ints.positive;
        description = "Output height in pixels.";
      };

      refresh = mkOption {
        type = types.ints.positive;
        description = "Refresh rate in millihertz.";
      };
    };
  };

  hookModule = types.submodule {
    options = {
      command = mkOption {
        type = types.str;
        description = "Command to execute for this hook.";
      };

      args = mkOption {
        type = with types; listOf str;
        default = [ ];
        description = "Command-line arguments passed to the hook.";
      };

      timeoutSecs = mkOption {
        type = types.ints.positive;
        default = 30;
        description = "Hook timeout in seconds.";
      };
    };
  };

  outputModule = types.submodule (
    { name, ... }:
    {
      options = {
        identity = mkOption {
          type = identityModule;
          default = { connector = name; };
          defaultText = literalExpression "{ connector = \"${name}\"; }";
          description = ''
            Output identity to persist alongside the layout entry. When unset,
            the output name is used as the connector.
          '';
        };

        enabled = mkOption {
          type = types.bool;
          default = false;
          description = "Whether this output should be enabled.";
        };

        mode = mkOption {
          type = types.nullOr modeModule;
          default = null;
          description = "Requested mode for the output.";
        };

        position = mkOption {
          type = positionModule;
          default = { };
          description = "Requested output position.";
        };

        scale = mkOption {
          type = types.float;
          default = 1.0;
          description = "Requested scale factor.";
        };

        transform = mkOption {
          type = types.enum [
            "normal"
            "90"
            "180"
            "270"
            "flipped"
            "flipped-90"
            "flipped-180"
            "flipped-270"
          ];
          default = "normal";
          description = "Requested output transform.";
        };

        mirrorTarget = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Optional mirror target connector name.";
        };

        backendData = mkOption {
          type = types.nullOr jsonFormat.type;
          default = null;
          description = "Backend-specific JSON payload to persist for this output.";
        };
      };
    }
  );

  matcherModule = types.submodule {
    options = {
      identity = mkOption {
        type = identityModule;
        default = { };
        description = "Identity fields used to match an output.";
      };

      required = mkOption {
        type = types.bool;
        default = false;
        description = "Whether the output must be present for the profile to match.";
      };

      positionHint = mkOption {
        type = types.nullOr positionModule;
        default = null;
        description = "Optional position hint used while matching outputs.";
      };
    };
  };

  hooksModule = types.submodule {
    options = {
      preApply = mkOption {
        type = with types; listOf hookModule;
        default = [ ];
        description = "Hooks run before applying a layout.";
      };

      postApply = mkOption {
        type = with types; listOf hookModule;
        default = [ ];
        description = "Hooks run after applying a layout.";
      };

      onFailure = mkOption {
        type = with types; listOf hookModule;
        default = [ ];
        description = "Hooks run after a failed apply attempt.";
      };
    };
  };

  profileOptionsModule = types.submodule {
    options = {
      ignoreScale = mkOption {
        type = types.bool;
        default = false;
        description = "Ignore scale changes when comparing a profile to current outputs.";
      };

      ignoreTransform = mkOption {
        type = types.bool;
        default = false;
        description = "Ignore transform changes when comparing a profile to current outputs.";
      };

      fallback = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Fallback profile name to try when this profile cannot be applied.";
      };
    };
  };

  profileModule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Profile name shown by the `waytorandr` CLI.";
      };

      priority = mkOption {
        type = types.ints.between 0 4294967295;
        default = 0;
        description = "Priority used when multiple profiles match the same setup.";
      };

      matchRules = mkOption {
        type = with types; listOf matcherModule;
        default = [ ];
        description = "Optional output match rules for this profile.";
      };

      layout = mkOption {
        type = with types; attrsOf outputModule;
        default = { };
        description = "Output layout keyed by connector name.";
        example = literalExpression ''
          {
            "eDP-1" = {
              enabled = true;
              scale = 2.0;
              mode = {
                width = 2880;
                height = 1800;
                refresh = 60000;
              };
            };
          }
        '';
      };

      hooks = mkOption {
        type = hooksModule;
        default = { };
        description = "Lifecycle hooks attached to this profile.";
      };

      options = mkOption {
        type = profileOptionsModule;
        default = { };
        description = "Profile comparison and fallback behavior.";
      };
    };
  };

  renderIdentity =
    identity:
    {
      edid_hash = identity.edidHash;
      make = identity.make;
      model = identity.model;
      serial = identity.serial;
      connector = identity.connector;
      description = identity.description;
      is_virtual = identity.isVirtual;
      is_ignored = identity.isIgnored;
    };

  renderPosition = position: {
    inherit (position) x y;
  };

  renderMode =
    mode:
    if mode == null then
      null
    else
      {
        inherit (mode) width height refresh;
      };

  renderHook = hook: {
    command = hook.command;
    args = hook.args;
    timeout_secs = hook.timeoutSecs;
  };

  renderOutput =
    name: output:
    {
      identity = renderIdentity (
        output.identity
        // lib.optionalAttrs (output.identity.connector == null) { connector = name; }
      );
      enabled = output.enabled;
      mode = renderMode output.mode;
      position = renderPosition output.position;
      scale = output.scale;
      transform = output.transform;
      mirror_target = output.mirrorTarget;
      backend_data = output.backendData;
    };

  renderMatchRule =
    matcher:
    {
      identity = renderIdentity matcher.identity;
      required = matcher.required;
      position_hint =
        if matcher.positionHint == null then null else renderPosition matcher.positionHint;
    };

  renderHooks = hooks: {
    pre_apply = map renderHook hooks.preApply;
    post_apply = map renderHook hooks.postApply;
    on_failure = map renderHook hooks.onFailure;
  };

  renderProfileOptions = profileOptions: {
    ignore_scale = profileOptions.ignoreScale;
    ignore_transform = profileOptions.ignoreTransform;
    fallback = profileOptions.fallback;
  };

  renderProfile = profile: {
    name = profile.name;
    priority = profile.priority;
    match_rules = map renderMatchRule profile.matchRules;
    layout = lib.mapAttrs renderOutput profile.layout;
    hooks = renderHooks profile.hooks;
    options = renderProfileOptions profile.options;
  };

  generatedProfiles = jsonFormat.generate "waytorandr-profiles.json" {
    profiles = map renderProfile cfg.profiles;
  };

  managedProfiles = cfg.profiles != [ ] || cfg.profilesFile != null;
in
{
  options.services.waytorandr = {
    enable = mkEnableOption "waytorandr and its automatic `waytorandrd` daemon";

    package = mkOption {
      type = types.package;
      default = defaultPackage;
      defaultText = literalExpression "inputs.waytorandr.packages.${pkgs.system}.default";
      description = "Package providing the `waytorandr` and `waytorandrd` binaries.";
    };

    profiles = mkOption {
      type = with types; listOf profileModule;
      default = [ ];
      description = ''
        Declarative profiles written to `~/.config/waytorandr/profiles.json`.
        Leave this empty to keep managing profiles imperatively with the CLI.
      '';
      example = literalExpression ''
        [
          {
            name = "laptop";
            layout = {
              "eDP-1" = {
                enabled = true;
                scale = 2.0;
                mode = {
                  width = 2880;
                  height = 1800;
                  refresh = 60000;
                };
              };
            };
          }
        ]
      '';
    };

    profilesFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = ''
        Existing `profiles.json` file to install instead of generating one from
        `services.waytorandr.profiles`.
      '';
    };

    environment = mkOption {
      type = with types; attrsOf str;
      default = { };
      description = "Environment variables passed to the `waytorandrd` user service.";
      example = {
        RUST_LOG = "waytorandrd=debug";
      };
    };

    systemdTarget = mkOption {
      type = types.str;
      default = config.wayland.systemd.target;
      defaultText = literalExpression "config.wayland.systemd.target";
      description = "Systemd target that should own the `waytorandrd` user service.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      (lib.hm.assertions.assertPlatform "services.waytorandr" pkgs lib.platforms.linux)
      {
        assertion = cfg.profiles == [ ] || cfg.profilesFile == null;
        message = "Use either services.waytorandr.profiles or services.waytorandr.profilesFile, not both.";
      }
      {
        assertion = lib.all (profile: profile.layout != { }) cfg.profiles;
        message = "Each services.waytorandr.profiles entry must define a non-empty layout.";
      }
    ];

    home.packages = [ cfg.package ];

    xdg.configFile."waytorandr/profiles.json" = mkIf managedProfiles {
      source = if cfg.profilesFile != null then cfg.profilesFile else generatedProfiles;
    };

    systemd.user.services.waytorandrd = {
      Unit = {
        Description = "Wayland display profile daemon";
        Documentation = "https://github.com/jsg/waytorandr";
        ConditionEnvironment = "WAYLAND_DISPLAY";
        PartOf = [ cfg.systemdTarget ];
        Requires = [ cfg.systemdTarget ];
        After = [ cfg.systemdTarget ];
        X-Restart-Triggers = mkIf managedProfiles [
          "${config.xdg.configFile."waytorandr/profiles.json".source}"
        ];
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
