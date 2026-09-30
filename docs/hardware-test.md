# Erster Test auf echter Hardware (M2)

Der DRM-Backend-Code (`src/udev.rs`) wurde in der Cloud nur gebaut und mit Unit-Tests
für die Moduswahl geprüft; ein laufendes Bild gab es dort nicht (kein `/dev/dri`).
Bitte deshalb zuerst so testen, dass ein Fehlschlag folgenlos bleibt.

## Voraussetzungen

- NVIDIA: Kernel-Parameter `nvidia-drm.modeset=1` (und `nvidia-drm.fbdev=1` ab Treiber 560).
  Ohne Modeset findet der Compositor keine nutzbare GPU.
- Eine Seat-Verwaltung: `seatd` (Dienst läuft, Benutzer in Gruppe `seat`) oder logind.
- Ein zweites Terminal/TTY oder SSH zum Beenden (`Ctrl+Alt+F<n>` wechselt zurück).
- Bauen: `cargo build --release` (Systempakete siehe README).

## Start

Von einer freien TTY (nicht aus einer Grafiksitzung):

```sh
RUST_LOG=info MYWM_CONFIG=$HOME/.config/mywm/config.toml \
  ./target/release/mywm-compositor "kitty" 2>compositor.log
```

Mit `MYWM_BACKEND=drm` lässt sich die Backend-Wahl erzwingen. Beenden: `Super+m`
(Standard-Binding `exit`), oder per SSH `pkill -TERM mywm-compositor`.

## Monitore

Statt kanshi gibt es `[[outputs]]` in der Config (`crates/mywm-config/example.toml`).
Für den Aufbau aus `config/kanshi.conf` von MyWM:

```toml
[[outputs]]
name = "HDMI-A-1"
mode = "2560x1080@60"
position = [0, 0]

[[outputs]]
name = "DP-3"
mode = "2560x1440@143.97"
position = [0, 1080]

[[outputs]]
name = "DP-1"
mode = "2560x1440@59.95"
position = [2560, 0]
transform = "270"
```

Im Log steht pro Monitor eine Zeile `DP-3: 2560x1440@143.97 scale 1`. Fehlt ein
Monitor, steht dort der Grund (kein CRTC, Modus nicht angeboten, Surface-Fehler).

## Was zu prüfen ist

1. Bild auf allen Monitoren, Position und Drehung wie konfiguriert.
2. Maus (Bewegung über Monitorgrenzen, Fokus folgt der Maus), Tastatur, Layout `de`.
3. Fenster starten (`Super+Return`), Workspaces `Super+1..9`, Monitorwechsel
   `Super+Alt+Pfeil`, Fenster verschieben `Super+Shift+Pfeil`.
4. VT-Wechsel `Ctrl+Alt+F2` und zurück: Bild und Eingabe müssen wiederkommen.
5. Monitor aus-/einstecken: Workspaces wandern zum ersten Monitor und zurück.
6. Ein Vollbild-Spiel/Video: läuft flüssig, kein Tearing (Tearing/VRR sind noch nicht
   eingebaut, siehe M3).

## Bekannte Lücken (kommen in M3/M4)

- Direct Scanout wird bereits von `DrmCompositor` versucht, ist aber ungetestet;
  Tearing-Control, VRR, Presentation-Time und explizite Synchronisation (`linux-drm-syncobj`,
  wichtig für NVIDIA) fehlen noch.
- XWayland (viele Spiele), Layer-Shell (Quickshell-Bar), Screencast und Sperrbildschirm fehlen.
- Nur die primäre GPU rendert; Ausgänge an anderen GPUs werden ignoriert.
- Der Cursor kommt aus dem xcursor-Theme (`XCURSOR_THEME`, `XCURSOR_SIZE`), animierte Cursor
  stehen still.

## Fehler melden

Bitte `compositor.log` (mit `RUST_LOG=debug` für Ausgaben von `smithay`) und die Ausgabe von
`dmesg | grep -i -E "nvidia|drm"` mitschicken.
