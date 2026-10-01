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
  # `logging` is the part that differs between the normal and the dev session.
  launcher = name: logging: pkgs.writeShellScript "${name}-launch" ''
    export XDG_CURRENT_DESKTOP=mywm
    export XDG_SESSION_DESKTOP=mywm
    export XDG_SESSION_TYPE=wayland
    export MYWM_SHELL_DIR=${cfg.package}/share/mywm/quickshell
    export MYWM_POLKIT_AGENT=${pkgs.polkit_gnome}/libexec/polkit-gnome-authentication-agent-1
    ${lib.optionalString (cfg.greeterDirectory != null) ''
      export MYWM_GREETER_DIR=${lib.escapeShellArg cfg.greeterDirectory}
    ''}
    export MYWM_CONFIG="''${MYWM_CONFIG:-''${XDG_CONFIG_HOME:-$HOME/.config}/mywm/config.toml}"
    ${logging}
    exec ${cfg.package}/bin/mywm-session
  '';

  # Warnings, errors and crashes only, in compositor.log (a crash must leave a trace).
  quiet = ''
    export RUST_LOG="''${RUST_LOG:-warn}"
  '';

  # Everything the session prints (also the helpers before the compositor has the screen)
  # goes to session.log, plus the compositor's frame statistics every 5 s per output.
  verbose = ''
    state="''${XDG_STATE_HOME:-$HOME/.local/state}/mywm"
    mkdir -p "$state"
    mv -f "$state/session.log" "$state/session.log.1" 2>/dev/null
    exec > "$state/session.log" 2>&1
    echo "$(date --iso-8601=ns) session launch, pid $$"
    export RUST_LOG="''${RUST_LOG:-info,perf=debug,smithay::xwayland::xwm=warn}"
  '';

  sessionPackage = name: title: comment: logging:
    pkgs.runCommand "${name}-wayland-session" { passthru.providedSessions = [ name ]; } ''
      mkdir -p $out/share/wayland-sessions
      cat > $out/share/wayland-sessions/${name}.desktop <<EOF
      [Desktop Entry]
      Name=${title}
      Comment=${comment}
      Exec=${launcher name logging}
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

    settingsPackage = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.mywm-settings;
      defaultText = lib.literalExpression "inputs.mywm.packages.\${pkgs.stdenv.hostPlatform.system}.mywm-settings";
      description = "The settings editor (`mywm-settings`).";
    };

    devSession = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Also offer the session "mywm (Dev)": the same compositor, but with the whole session's
        output in `~/.local/state/mywm/session.log` and frame statistics (`perf`) in the log.
        The normal session only logs warnings and errors.
      '';
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
    environment.systemPackages = [ cfg.package cfg.settingsPackage ];
    services.displayManager.sessionPackages =
      [ (sessionPackage "mywm" "mywm" "Smithay-based tiling compositor with its own Quickshell shell" quiet) ]
      ++ lib.optional cfg.devSession
        (sessionPackage "mywm-dev" "mywm (Dev)" "mywm with session log and frame statistics" verbose);
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
