# Installation und Sitzung

Der Compositor läuft als Sitzung (vom Display-Manager gestartet) oder nested in einem anderen
Fenster zum Entwickeln (`cargo run`). Die Sitzung besteht aus `splice-session` (Skript): es startet den
Compositor und darin einen Polkit-Agenten und die Portal-Umgebung. Die Shell (Bar, Launcher, Picker,
Sperrbildschirm) zeichnet der Compositor selbst; Idle startet er ebenfalls selbst (swayidle).

## NixOS (Flake)

Flake, Paket und NixOS-Modul (`flake.nix`, `nix/package.nix`, `nix/module.nix`) sind im
täglichen Gebrauch. Das Modul legt die Sitzung „splice“ an (`programs.splice`).

```nix
# flake.nix des Systems
inputs.splice.url = "github:Chr1ssi/Splice-Compositor";
# ...
modules = [
  inputs.splice.nixosModules.default
  {
    programs.splice.enable = true;
    # Optional: zusätzlich die Sitzung „splice (Dev)“ (siehe unten).
    programs.splice.devSession = true;
  }
];
```

Das Modul legt die Sitzung „splice“ für den Display-Manager an, aktiviert Xwayland, PipeWire und
die Portale (Screen-Sharing über `splice-portal`, Dateiauswahl über das GTK-Portal) und den
PAM-Dienst `splice-lock` für den Sperrbildschirm. Die Konfiguration liegt in `~/.config/splice/config.toml`
(Vorlage: `crates/splice-config/example.toml`). NVIDIA: `hardware.nvidia.modesetting.enable = true`
und der offene Kernelmodul-Treiber wie bisher.

Umstieg von mywm (so hieß das Projekt bis Oktober 2026): Pfade, Umgebungsvariablen (`MYWM_*` →
`SPLICE_*`), Crates, Binaries, die Sitzung, der PAM-Dienst und `programs.mywm` heißen jetzt splice.
Nur zwei Dinge werden übernommen: Fehlt `~/.config/splice/config.toml`, liest der Compositor noch
`~/.config/mywm/config.toml`, und `~/.local/state/mywm` zieht beim ersten Start nach
`~/.local/state/splice` um (Wallpaper-Wahl, Theme, Nachtlicht, Fensterspeicher). Beides entfällt,
sobald keine Installation mehr darauf angewiesen ist.

Anmeldung: Mit `programs.splice.greeter.enable = true` (und `programs.splice.greeterDirectory`)
richtet das Modul greetd mit dem eingebauten Anmeldebildschirm ein. Er läuft in
`splice-compositor --greeter "<Befehl>"`, der endet, sobald der Anmeldebildschirm endet, damit
greetd die gewählte Sitzung startet. `greeter.user` füllt den Benutzernamen vor,
`greeter.settings` ist die splice-Konfiguration dafür (Ausgänge, Tastatur, Eingabe; Xwayland und
die Idle-Sperre sind aus), `greeter.environment` setzt z. B. das Cursor-Theme. Wallpaper und
Farben (`theme.json`) spiegelt die Sitzung bei jedem Themewechsel in `greeterDirectory`. Mit
Plymouth beendet greetd den Bootsplash selbst (`plymouth quit --retain-splash`), so bleibt dessen
letztes Bild stehen, bis splice den Bildschirm übernimmt, statt eines schwarzen Bildschirms.
Den Anmeldebildschirm zeichnet der Compositor selbst (`splice-compositor --greeter`, siehe `docs/shell.md`).

Logging: Die Sitzung „splice“ schreibt nur Warnungen, Fehler und Abstürze nach
`~/.local/state/splice/compositor.log`. Mit `programs.splice.devSession = true` gibt es zusätzlich
„splice (Dev)“: derselbe Compositor, aber die gesamte Ausgabe der Sitzung (auch der Helfer) landet in
`~/.local/state/splice/session.log`, und alle 5 s steht pro Ausgang eine Zeile mit Frames und
Renderzeit im Log (`RUST_LOG=info,perf=debug`). Von beiden Dateien bleibt die der vorigen Sitzung
als `*.1` erhalten.

## Andere Distributionen

1. Bauen: `cargo build --release -p splice-compositor -p splice-portal -p splice-shell-services` (Abhängigkeiten siehe README).
2. Installieren: `splice-compositor`, `splice-portal` und `splice-shell-services` (die eingebaute Shell sucht es neben dem
   Compositor, sonst im `PATH`) in den `PATH`; `scripts/splice-session` und
   `scripts/session-environment` ebenfalls (`SPLICE_BINARY` und `SPLICE_SESSION_ENVIRONMENT` zeigen
   sonst auf `splice-compositor` und `session-environment` im `PATH`).
3. Session-Eintrag `/usr/share/wayland-sessions/splice.desktop`:
   ```ini
   [Desktop Entry]
   Name=splice
   Exec=splice-session
   Type=Application
   DesktopNames=splice
   ```
4. Portale: Dateien aus `portal/` installieren (siehe `docs/streaming.md`), für
   `XDG_CURRENT_DESKTOP=splice` als `~/.config/xdg-desktop-portal/splice-portals.conf`.
5. Laufzeit-Programme: `swayidle`, `wlopm`, `wl-clipboard` (Screenshot in
   die Zwischenablage), `libnotify` (Benachrichtigung), `xwayland`, `polkit-gnome` (optional),
   `wireplumber` (`wpctl`: Lautstärke der eingebauten Bar), `fontconfig` (`fc-match`: Schrift der Formularfelder der
   eingebauten Shell).

## Erster Start auf der Hardware

1. Testweise von einer zweiten TTY: `splice-session` (oder nur `splice-compositor`).
2. Log: `~/.local/state/splice/compositor.log`; Anleitung und Checkliste in `docs/hardware-test.md`.
3. Nützliche Schalter: `RUST_LOG=info,perf=debug`, `[render]`, `[effects]`, `[input]` in der Config.
