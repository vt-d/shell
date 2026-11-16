self:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.desktop-shell;
  servicePath = lib.concatStringsSep ":" (
    lib.optional (cfg.extraPackages != [ ]) (lib.makeBinPath cfg.extraPackages)
    ++ [ "${config.home.profileDirectory}/bin" "/run/current-system/sw/bin" "/usr/local/bin" "/usr/bin" "/bin" ]
  );
in
{
  options.services.desktop-shell = {
    enable = lib.mkEnableOption "the native Rust desktop shell";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "inputs.shell.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "Desktop shell package to run.";
    };
    extraPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
      description = "Additional programs available to the shell and applications it launches.";
    };
    matugen = {
      enable = lib.mkEnableOption "Matugen configuration for the shell";
      settings = lib.mkOption {
        type = (pkgs.formats.toml { }).type;
        default = { };
        description = "Additional settings merged into matugen/config.toml.";
      };
    };
    systemdTarget = lib.mkOption {
      type = lib.types.str;
      default = "graphical-session.target";
      example = "hyprland-session.target";
      description = "User session target that starts and stops the shell.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.isLinux;
        message = "services.desktop-shell requires Linux and a Wayland session.";
      }
    ];
    # The template is immutable; Matugen's output (shell/theme.json) is writable.
    xdg.configFile."matugen/templates/shell.json".source = ../matugen/shell.json;
    xdg.configFile."matugen/config.toml" = lib.mkIf cfg.matugen.enable {
      source = (pkgs.formats.toml { }).generate "matugen-config.toml"
        (lib.recursiveUpdate cfg.matugen.settings {
          templates.shell = {
            input_path = "${config.xdg.configHome}/matugen/templates/shell.json";
            output_path = "${config.xdg.configHome}/shell/theme.json";
          };
        });
    };
    home.packages = [ cfg.package ] ++ lib.optional cfg.matugen.enable pkgs.matugen;
    home.activation.desktopShellThemeDirectory = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
      run mkdir -p ${lib.escapeShellArg "${config.xdg.configHome}/shell"}
    '';
    systemd.user.services.desktop-shell = {
      Unit = {
        Description = "Native Rust desktop shell";
        After = [ "graphical-session-pre.target" ];
        PartOf = [ cfg.systemdTarget ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
      };
      Service = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
        RestartSec = 2;
        Environment = [
          "PATH=${servicePath}"
          "XDG_CONFIG_HOME=${config.xdg.configHome}"
          "XDG_DATA_HOME=${config.xdg.dataHome}"
          "XDG_CACHE_HOME=${config.xdg.cacheHome}"
          "XDG_STATE_HOME=${config.xdg.stateHome}"
        ];
      };
      Install.WantedBy = [ cfg.systemdTarget ];
    };
  };
}
