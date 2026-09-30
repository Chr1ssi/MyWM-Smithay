{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.mywm;

  # The session script with its environment; the display manager starts `mywm-session`.
  session = pkgs.writeShellScriptBin "mywm-session-launch" ''
    export XDG_CURRENT_DESKTOP=mywm
    export XDG_SESSION_DESKTOP=mywm
    export XDG_SESSION_TYPE=wayland
    export MYWM_SHELL_DIR=${cfg.package}/share/mywm/quickshell
    export MYWM_POLKIT_AGENT=${pkgs.polkit_gnome}/libexec/polkit-gnome-authentication-agent-1
    ${lib.optionalString (cfg.greeterDirectory != null) ''
      export MYWM_GREETER_DIR=${lib.escapeShellArg cfg.greeterDirectory}
    ''}
    export MYWM_CONFIG="''${MYWM_CONFIG:-''${XDG_CONFIG_HOME:-$HOME/.config}/mywm/config.toml}"
    exec ${cfg.package}/bin/mywm-session
  '';

  sessionPackage = pkgs.runCommand "mywm-wayland-session" {
    passthru.providedSessions = [ "mywm" ];
  } ''
    mkdir -p $out/share/wayland-sessions
    cat > $out/share/wayland-sessions/mywm.desktop <<EOF
    [Desktop Entry]
    Name=mywm
    Comment=Smithay-based tiling compositor with its own Quickshell shell
    Exec=${session}/bin/mywm-session-launch
    Type=Application
    DesktopNames=mywm
    Keywords=tiling;wayland;compositor;
    EOF
  '';
in
{
  options.programs.mywm = {
    enable = lib.mkEnableOption "the mywm compositor session";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "inputs.mywm.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The mywm package to use.";
    };

    greeterDirectory = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Directory writable by the session user and readable by the greeter.
        On each theme change mywm writes `background` (the wallpaper) and
        `theme.css` (GTK color definitions) there.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    services.displayManager.sessionPackages = [ sessionPackage ];
    security.pam.services.swaylock = { };

    # Started by session-environment; pulls in graphical-session.target so systemd
    # user services bound to it run inside the session.
    systemd.user.targets.mywm-session = {
      description = "mywm compositor session";
      bindsTo = [ "graphical-session.target" ];
      wants = [ "graphical-session-pre.target" ];
      after = [ "graphical-session-pre.target" ];
      before = [ "graphical-session.target" ];
    };
    programs.xwayland.enable = true;

    # Screen sharing goes through mywm-portal (the compositor asks what to share); the
    # file chooser and the rest come from the GTK portal.
    xdg.portal = {
      enable = true;
      extraPortals = [ pkgs.xdg-desktop-portal-gtk cfg.package ];
      config.mywm = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "mywm" ];
      };
    };
    services.pipewire.enable = lib.mkDefault true;
  };
}
