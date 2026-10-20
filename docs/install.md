# Installation und Sitzung

Der Compositor läuft als Sitzung (vom Display-Manager gestartet) oder nested in einem anderen
Fenster zum Entwickeln (`cargo run`). Die Sitzung besteht aus `mywm-session` (Skript): es startet den
Compositor und darin Wallpaper-Picker, Bar, einen Polkit-Agenten und die Portal-Umgebung. Sperre und
Idle startet der Compositor selbst (Sperrbildschirm `lock.qml` aus mywm-shell, swayidle).

## NixOS (Flake)

Flake, Paket und NixOS-Modul (`flake.nix`, `nix/package.nix`, `nix/module.nix`) sind im
täglichen Gebrauch. Das Modul legt die Sitzung „mywm“ an (`programs.mywm`).

```nix
# flake.nix des Systems
inputs.mywm.url = "github:Chr1ssi/MyWM-Smithay";
# ...
modules = [
  inputs.mywm.nixosModules.default
  {
    programs.mywm.enable = true;
    # Optional: zusätzlich die Sitzung „mywm (Dev)“ (siehe unten).
    programs.mywm.devSession = true;
  }
];
```

Das Modul legt die Sitzung „mywm“ für den Display-Manager an, aktiviert Xwayland, PipeWire und
die Portale (Screen-Sharing über `mywm-portal`, Dateiauswahl über das GTK-Portal) und den
PAM-Dienst `mywm-lock` für den Sperrbildschirm. Die Konfiguration liegt in `~/.config/mywm/config.toml`
(Vorlage: `crates/mywm-config/example.toml`). NVIDIA: `hardware.nvidia.modesetting.enable = true`
und der offene Kernelmodul-Treiber wie bisher.

Anmeldung: Mit `programs.mywm.greeter.enable = true` (und `programs.mywm.greeterDirectory`)
richtet das Modul greetd mit dem Anmeldebildschirm aus mywm-shell ein. Er läuft in
`mywm-compositor --greeter "<Befehl>"`, der endet, sobald der Anmeldebildschirm endet, damit
greetd die gewählte Sitzung startet. `greeter.user` füllt den Benutzernamen vor,
`greeter.settings` ist die mywm-Konfiguration dafür (Ausgänge, Tastatur, Eingabe; Xwayland und
die Idle-Sperre sind aus), `greeter.environment` setzt z. B. das Cursor-Theme. Wallpaper und
Farben (`theme.json`) spiegelt die Sitzung bei jedem Themewechsel in `greeterDirectory`. Mit
Plymouth beendet greetd den Bootsplash selbst (`plymouth quit --retain-splash`), so bleibt dessen
letztes Bild stehen, bis mywm den Bildschirm übernimmt, statt eines schwarzen Bildschirms.

Logging: Die Sitzung „mywm“ schreibt nur Warnungen, Fehler und Abstürze nach
`~/.local/state/mywm/compositor.log`. Mit `programs.mywm.devSession = true` gibt es zusätzlich
„mywm (Dev)“: derselbe Compositor, aber die gesamte Ausgabe der Sitzung (auch der Helfer) landet in
`~/.local/state/mywm/session.log`, und alle 5 s steht pro Ausgang eine Zeile mit Frames und
Renderzeit im Log (`RUST_LOG=info,perf=debug`). Von beiden Dateien bleibt die der vorigen Sitzung
als `*.1` erhalten.

## Andere Distributionen

1. Bauen: `cargo build --release -p mywm-compositor -p mywm-portal -p mywm-shell-services` (Abhängigkeiten siehe README).
2. Installieren: `mywm-compositor`, `mywm-portal` und `mywm-shell-services` (die eingebaute Shell sucht es neben dem
   Compositor, sonst im `PATH`) in den `PATH`; `scripts/mywm-session` und
   `scripts/session-environment` ebenfalls (`MYWM_BINARY` und `MYWM_SESSION_ENVIRONMENT` zeigen
   sonst auf `mywm-compositor` und `session-environment` im `PATH`).
3. Quickshell-Oberfläche: das Repo `mywm-shell` bereitstellen und `MYWM_SHELL_DIR` auf dessen
   `quickshell/`-Ordner setzen.
4. Session-Eintrag `/usr/share/wayland-sessions/mywm.desktop`:
   ```ini
   [Desktop Entry]
   Name=mywm
   Exec=mywm-session
   Type=Application
   DesktopNames=mywm
   ```
5. Portale: Dateien aus `portal/` installieren (siehe `docs/streaming.md`), für
   `XDG_CURRENT_DESKTOP=mywm` als `~/.config/xdg-desktop-portal/mywm-portals.conf`.
6. Laufzeit-Programme: `quickshell`, `swayidle`, `wlopm`, `wl-clipboard` (Screenshot in
   die Zwischenablage), `libnotify` (Benachrichtigung), `xwayland`, `polkit-gnome` (optional),
   `wireplumber` (`wpctl`: Lautstärke der eingebauten Bar).

## Erster Start auf der Hardware

1. Testweise von einer zweiten TTY: `mywm-session` (oder nur `mywm-compositor`).
2. Log: `~/.local/state/mywm/compositor.log`; Anleitung und Checkliste in `docs/hardware-test.md`.
3. Nützliche Schalter: `RUST_LOG=info,perf=debug`, `[render]`, `[effects]`, `[input]` in der Config.
