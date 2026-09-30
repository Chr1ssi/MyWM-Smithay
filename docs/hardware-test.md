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

## Spiele, Tearing und VRR (M3)

```toml
game_app_id_prefixes = ["steam_app_", "gamescope"]   # erkennt Spiele (auch deren Dialoge)
async_outputs = ["DP-3"]                              # Tearing erlaubt (nur mit Spiel im Vollbild)

[vrr]
enabled = true
output = "DP-3"
```

Im Log steht bei jedem Wechsel, warum VRR/Tearing (nicht) aktiv sind, z. B.
`DP-3: fullscreen game: true; tearing allowed by config: true; requested by the game: false`,
außerdem `a client allows tearing for one of its surfaces` (das Spiel bittet um Tearing) und
`DP-3: tearing on (immediate page flips)`. Fehlt die Zeile „allows tearing“, nutzt das Spiel oder der
Treiber `wp_tearing_control_v1` nicht; dann tearen wir nicht, auch wenn die Config es erlaubt.

Im Log erscheint beim Start des Spiels je nach Fall `DP-3: adaptive sync on` und, wenn der
Treiber ein sofortiges Kippen ablehnt, `tearing flip rejected (...); falling back to vsync`.
Beim GPU-Start steht `explicit sync (linux-drm-syncobj) available` oder der Grund, warum nicht.

Prüfen:

1. Spiel starten (Proton/Vulkan, `PROTON_ENABLE_WAYLAND=1` oder über XWayland – siehe unten):
   Vollbild deckt den Monitor komplett, keine Rahmen. Direct Scanout erkennt man daran, dass
   die GPU-Last im Vollbild kaum über den Wert ohne Compositor liegt; im Log mit
   `RUST_LOG=smithay::backend::drm=debug` erscheinen Zeilen zur Plane-Zuweisung.
2. VRR: Monitor-OSD bzw. `cat /sys/kernel/debug/dri/*/vrr_range` / Anzeige der Bildrate.
3. Tearing: braucht ein Spiel, das `wp_tearing_control_v1` nutzt (z. B. ein Wayland-natives
   Vulkan-Spiel mit `MESA_VK_WSI_PRESENT_MODE=immediate`; NVIDIA-Treiber setzen das je nach
   Version selbst).
4. Maus in Spielen: Mauszeiger sperrt sich (`pointer-constraints`) und liefert rohe Deltas
   (`relative-pointer`); Spiele unter XWayland brauchen dafür M4.

## Desktop-Integration (M4)

- **Bar/Launcher**: Quickshell (`mywm-shell`) über `wlr-layer-shell`; die Bar spricht über
  `$MYWM_SOCKET` mit dem Compositor. Ohne gesetztes `MYWM_SOCKET` in der Sitzung bleibt sie leer.
- **Sperre/Idle**: `Super+Escape` startet swaylock; auf Hardware startet der Compositor
  außerdem `swayidle` (Sperre nach `[idle] lock_after_seconds`, danach Monitore aus über
  `wlopm`). Dafür müssen `swaylock`, `swayidle` und `wlopm` installiert sein.
- **Steam/X11**: `steam` läuft über Xwayland (`DISPLAY` setzt der Compositor für gestartete
  Programme selbst). Nativ: Proton mit `PROTON_ENABLE_WAYLAND=1`, SDL mit `SDL_VIDEODRIVER=wayland`.
- **Streaming**: siehe `docs/streaming.md`.

## Frame-Pacing und Latenz (M5)

Logs: mit `RUST_LOG=info,perf=debug` erscheint alle 5 s pro Ausgang eine Zeile
`… frames in …s, cpu render avg … ms, worst … ms, N slower than the refresh interval`
(CPU-Zeit für Rendern und Einreihen des Frames, nicht GPU-Zeit). Läuft ein Spiel
fullscreen, meldet der Compositor bei Wechsel `direct scanout (no compositing)` bzw.
`composited` (Info-Level).

Späte Frame-Planung (Standard: aus) zum A/B-Vergleich in der `config.toml`:

```toml
[render]
late_scheduling = true
margin_ms = 2.0   # Reserve vor dem Vblank zusätzlich zur gemessenen Renderzeit
```

Der Frame wird dann erst kurz vor dem Vblank gerendert (so zeigt er den neuesten Client-Inhalt)
und die Frame-Callbacks gehen am Vblank raus. Aktiv nur bei festem Refresh (nicht bei VRR/Tearing).
Hilfreich: zuerst die `perf`-Zeilen ansehen; ist `worst` nahe am Refresh-Intervall, `margin_ms`
erhöhen oder die Option auslassen. Bitte Eindruck (Latenz/Ruckler) und die `perf`-Zeilen zurückmelden.

## Bekannte Lücken

- Tearing setzt einen Kernel mit atomaren Async-Flips (Linux ≥ 6.8) und Treiberunterstützung voraus;
  die dafür nötige kleine Änderung an smithay steckt in `vendor/` (siehe `vendor/README.md`).
- Fensteraufnahme über das Portal (nur Monitore) und der Wallpaper-Picker fehlen noch.
- Der Cursor kommt aus dem xcursor-Theme (`XCURSOR_THEME`, `XCURSOR_SIZE`), animierte Cursor
  stehen still.

## Logs

Der Compositor schreibt jetzt immer eine Logdatei: `~/.local/state/mywm/compositor.log`
(Pfad mit `MYWM_LOG_FILE` änderbar, `MYWM_LOG_FILE=off` schaltet sie ab; die Datei der vorigen
Sitzung bleibt als `compositor.log.1` liegen). Darin stehen Start, Config-Pfad, jeder Monitor mit
Position und Größe, GPU und Modus, neue Fenster mit App-ID, Layer-Surfaces (Bar, Wallpaper),
Xwayland, VRR/Tearing und jeder Client, der wegen eines Protokollfehlers rausgeworfen wurde.
Mit `RUST_LOG=debug` (oder z. B. `RUST_LOG=info,smithay::backend::drm=debug`) wird es
ausführlicher. Panics landen ebenfalls dort.

## Fehler melden

Bitte `compositor.log` (mit `RUST_LOG=debug` für Ausgaben von `smithay`) und die Ausgabe von
`dmesg | grep -i -E "nvidia|drm"` mitschicken.
