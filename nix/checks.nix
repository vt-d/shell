{ pkgs, self, home-manager }:
let
  inherit (pkgs) lib;
  evaluate = settings: (home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      self.homeManagerModules.default
      {
        home.username = "test-user";
        home.homeDirectory = "/home/test-user";
        home.stateVersion = "26.05";
        services.desktop-shell = settings;
      }
    ];
  }).config;
  disabled = evaluate { enable = false; };
  enabled = evaluate { enable = true; };
  custom = evaluate {
    enable = true;
    package = pkgs.hello;
    systemdTarget = "hyprland-session.target";
    extraPackages = [ pkgs.foot ];
    matugen = {
      enable = true;
      settings.config.caching = true;
    };
  };
  service = custom.systemd.user.services.desktop-shell;
in
assert !(disabled.systemd.user.services ? desktop-shell);
assert !(disabled.xdg.configFile ? "matugen/templates/shell.json");
assert enabled.services.desktop-shell.package == self.packages.${pkgs.stdenv.hostPlatform.system}.default;
assert enabled.systemd.user.services.desktop-shell.Install.WantedBy == [ "graphical-session.target" ];
assert !(enabled.xdg.configFile ? "shell/theme.json");
assert service.Service.ExecStart == [ (lib.getExe pkgs.hello) ];
assert service.Install.WantedBy == [ "hyprland-session.target" ];
assert service.Unit.PartOf == [ "hyprland-session.target" ];
assert lib.all (a: a.assertion) custom.assertions;
pkgs.runCommand "desktop-shell-home-manager-check" { } ''
  test -f ${custom.xdg.configFile."matugen/templates/shell.json".source}
  grep -F 'caching = true' ${custom.xdg.configFile."matugen/config.toml".source}
  grep -F '/home/test-user/.config/shell/theme.json' ${custom.xdg.configFile."matugen/config.toml".source}
  grep -F 'WantedBy=hyprland-session.target' ${custom.xdg.configFile."systemd/user/desktop-shell.service".source}
  grep -F 'ConditionEnvironment=WAYLAND_DISPLAY' ${custom.xdg.configFile."systemd/user/desktop-shell.service".source}
  grep -F '${lib.makeBinPath [ pkgs.foot ]}' ${custom.xdg.configFile."systemd/user/desktop-shell.service".source}
  touch "$out"
''
