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
  greeterConfig = (pkgs.formats.toml { }).generate "mywm-greeter.toml" (
    {
      xwayland = false;
      idle = {
        lock_after_seconds = 0;
        monitor_off_after_seconds = 0;
      };
    }
    // cfg.greeter.settings
  );

  # greetd's session for the login screen: mywm with nothing but its login screen; it ends once a session starts.
  greeterLaunch = pkgs.writeShellScript "mywm-greeter" ''
    export XDG_SESSION_TYPE=wayland
    export MYWM_CONFIG=${greeterConfig}
    export MYWM_THEME_STATE=${lib.escapeShellArg cfg.greeterDirectory}/theme.json
    export MYWM_GREETER_BACKGROUND=${lib.escapeShellArg cfg.greeterDirectory}/background
    export MYWM_GREETER_SESSIONS=${config.services.displayManager.sessionData.desktops}/share/wayland-sessions
    ${lib.optionalString (cfg.greeter.user != null) "export MYWM_GREETER_USER=${lib.escapeShellArg cfg.greeter.user}"}
    # The greeter user has no state directory; messages go to the journal.
    export MYWM_LOG_FILE=off RUST_LOG=warn
    ${lib.concatStringsSep "\n" (lib.mapAttrsToList (name: value: "export ${name}=${lib.escapeShellArg value}") cfg.greeter.environment)}
    exec ${pkgs.dbus}/bin/dbus-run-session ${cfg.package}/bin/mywm-compositor --greeter
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
        On each theme change mywm writes `background` (the wallpaper),
        `theme.json` (the colors, for mywm's login screen) and `theme.css`
        (GTK color definitions) there.
      '';
    };

    greeter = {
      enable = lib.mkEnableOption ''
        greetd with mywm's own login screen (in the look of the lock screen), shown by
        `mywm-compositor --greeter`. Needs `greeterDirectory` for the wallpaper and colors'';

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
          mywm configuration while the login screen shows (`config.toml` keys): outputs, keyboard,
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
        message = "programs.mywm.greeter needs programs.mywm.greeterDirectory (wallpaper and colors).";
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

    # Plymouth's last picture stays until mywm takes the display, instead of a black screen while the login
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
      [ (sessionPackage "mywm" "mywm" "Smithay-based tiling compositor with its built-in shell" quiet) ]
      ++ lib.optional cfg.devSession
        (sessionPackage "mywm-dev" "mywm (Dev)" "mywm with session log and frame statistics" verbose);
    # The built-in lock screen checks the password with this service.
    security.pam.services.mywm-lock = { };

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
    # mywm-portal.service, the session-bound unit behind the portal's D-Bus activation.
    systemd.packages = [ cfg.package ];

    # Screen sharing goes through mywm-portal (the compositor asks what to share), and so does
    # Inhibit (the GTK portal's needs a GNOME session); the file chooser and the rest come from
    # the GTK portal.
    xdg.portal = {
      enable = true;
      extraPortals = [ pkgs.xdg-desktop-portal-gtk cfg.package ];
      config.mywm = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "mywm" ];
        "org.freedesktop.impl.portal.Inhibit" = [ "mywm" ];
      };
    };
    services.pipewire.enable = lib.mkDefault true;
  };
}
