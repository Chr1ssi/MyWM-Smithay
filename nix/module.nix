{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.splice;

  # The session script with its environment; the display manager starts `splice-session`.
  # `logging` is the part that differs between the normal and the dev session.
  launcher = name: logging: pkgs.writeShellScript "${name}-launch" ''
    export XDG_CURRENT_DESKTOP=splice
    export XDG_SESSION_DESKTOP=splice
    export XDG_SESSION_TYPE=wayland
    export SPLICE_POLKIT_AGENT=${pkgs.polkit_gnome}/libexec/polkit-gnome-authentication-agent-1
    ${lib.optionalString (cfg.greeterDirectory != null) ''
      export SPLICE_GREETER_DIR=${lib.escapeShellArg cfg.greeterDirectory}
    ''}
    export SPLICE_CONFIG="''${SPLICE_CONFIG:-''${XDG_CONFIG_HOME:-$HOME/.config}/splice/config.toml}"
    ${logging}
    exec ${cfg.package}/bin/splice-session
  '';

  # Warnings, errors and crashes only, in compositor.log (a crash must leave a trace).
  quiet = ''
    export RUST_LOG="''${RUST_LOG:-warn}"
  '';

  # Everything the session prints (also the helpers before the compositor has the screen)
  # goes to session.log, plus the compositor's frame statistics every 5 s per output.
  verbose = ''
    state="''${XDG_STATE_HOME:-$HOME/.local/state}/splice"
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
      DesktopNames=splice
      Keywords=tiling;wayland;compositor;
      EOF
    '';
  greeterConfig = (pkgs.formats.toml { }).generate "splice-greeter.toml" (
    {
      xwayland = false;
      idle = {
        lock_after_seconds = 0;
        monitor_off_after_seconds = 0;
      };
    }
    // cfg.greeter.settings
  );

  # greetd's session for the login screen: splice with nothing but its login screen; it ends once a session starts.
  greeterLaunch = pkgs.writeShellScript "splice-greeter" ''
    export XDG_SESSION_TYPE=wayland
    export SPLICE_CONFIG=${greeterConfig}
    export SPLICE_THEME_STATE=${lib.escapeShellArg cfg.greeterDirectory}/theme.json
    export SPLICE_GREETER_BACKGROUND=${lib.escapeShellArg cfg.greeterDirectory}/background
    export SPLICE_GREETER_SESSIONS=${config.services.displayManager.sessionData.desktops}/share/wayland-sessions
    ${lib.optionalString (cfg.greeter.user != null) "export SPLICE_GREETER_USER=${lib.escapeShellArg cfg.greeter.user}"}
    # The greeter user has no state directory; messages go to the journal.
    export SPLICE_LOG_FILE=off RUST_LOG=warn
    ${lib.concatStringsSep "\n" (lib.mapAttrsToList (name: value: "export ${name}=${lib.escapeShellArg value}") cfg.greeter.environment)}
    exec ${pkgs.dbus}/bin/dbus-run-session ${cfg.package}/bin/splice-compositor --greeter
  '';
in
{
  options.programs.splice = {
    enable = lib.mkEnableOption "the splice compositor session";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "inputs.splice.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The splice package to use.";
    };

    settingsPackage = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.splice-settings;
      defaultText = lib.literalExpression "inputs.splice.packages.\${pkgs.stdenv.hostPlatform.system}.splice-settings";
      description = "The settings editor (`splice-settings`).";
    };

    devSession = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Also offer the session "splice (Dev)": the same compositor, but with the whole session's
        output in `~/.local/state/splice/session.log` and frame statistics (`perf`) in the log.
        The normal session only logs warnings and errors.
      '';
    };

    greeterDirectory = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Directory writable by the session user and readable by the greeter.
        On each theme change splice writes `background` (the wallpaper),
        `theme.json` (the colors, for splice's login screen) and `theme.css`
        (GTK color definitions) there.
      '';
    };

    greeter = {
      enable = lib.mkEnableOption ''
        greetd with splice's own login screen (in the look of the lock screen), shown by
        `splice-compositor --greeter`. Needs `greeterDirectory` for the wallpaper and colors'';

      user = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "User name the login screen starts with.";
      };

      settings = lib.mkOption {
        type = (pkgs.formats.toml { }).type;
        default = { };
        example = lib.literalExpression ''
          {
            keyboard.layout = "de";
            outputs = [ { name = "DP-3"; mode = "2560x1440@143.97"; position = [ 0 0 ]; } ];
          }
        '';
        description = ''
          splice configuration while the login screen shows (`config.toml` keys): outputs, keyboard,
          input. Xwayland and the idle lock are off there.
        '';
      };

      environment = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = { };
        example = { XCURSOR_THEME = "Bibata-Modern-Ice"; XCURSOR_SIZE = "24"; };
        description = "Further environment of the login screen, such as the cursor theme.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.greeter.enable || cfg.greeterDirectory != null;
        message = "programs.splice.greeter needs programs.splice.greeterDirectory (wallpaper and colors).";
      }
    ];

    services.greetd = lib.mkIf cfg.greeter.enable {
      enable = true;
      settings.default_session = {
        command = "${greeterLaunch}";
        user = "greeter";
      };
      # Plymouth ends only as greetd starts (below), not before it.
      greeterManagesPlymouth = config.boot.plymouth.enable;
    };

    # Plymouth's last picture stays until splice takes the display, instead of a black screen while the login
    # screen starts. Only root may tell Plymouth to quit, so greetd does it rather than the greeter; like GDM.
    systemd.services.greetd = lib.mkIf (cfg.greeter.enable && config.boot.plymouth.enable) {
      after = [ "plymouth-start.service" ];
      conflicts = [ "plymouth-quit.service" ];
      onFailure = [ "plymouth-quit.service" ];
      serviceConfig.ExecStartPre = [ "-${config.boot.plymouth.package}/bin/plymouth quit --retain-splash" ];
    };
    # Otherwise starting multi-user.target (nixos-rebuild switch) would stop greetd for plymouth-quit.
    systemd.services.plymouth-quit = lib.mkIf (cfg.greeter.enable && config.boot.plymouth.enable) {
      wantedBy = lib.mkForce [ ];
    };

    environment.systemPackages = [ cfg.package cfg.settingsPackage ];
    services.displayManager.sessionPackages =
      [ (sessionPackage "splice" "splice" "Smithay-based tiling compositor with its built-in shell" quiet) ]
      ++ lib.optional cfg.devSession
        (sessionPackage "splice-dev" "splice (Dev)" "splice with session log and frame statistics" verbose);
    # The built-in lock screen checks the password with this service.
    security.pam.services.splice-lock = { };

    # Started by session-environment; pulls in graphical-session.target so systemd
    # user services bound to it run inside the session.
    systemd.user.targets.splice-session = {
      description = "splice compositor session";
      bindsTo = [ "graphical-session.target" ];
      wants = [ "graphical-session-pre.target" ];
      after = [ "graphical-session-pre.target" ];
      before = [ "graphical-session.target" ];
    };
    programs.xwayland.enable = true;
    # splice-portal.service, the session-bound unit behind the portal's D-Bus activation.
    systemd.packages = [ cfg.package ];

    # Screen sharing goes through splice-portal (the compositor asks what to share), and so does
    # Inhibit (the GTK portal's needs a GNOME session); the file chooser and the rest come from
    # the GTK portal.
    xdg.portal = {
      enable = true;
      extraPortals = [ pkgs.xdg-desktop-portal-gtk cfg.package ];
      config.splice = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "splice" ];
        "org.freedesktop.impl.portal.Inhibit" = [ "splice" ];
      };
    };
    services.pipewire.enable = lib.mkDefault true;
  };
}
