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
  ./target/release/mywm-compositor "foot" 2>compositor.log
```

Mit `MYWM_BACKEND=drm` lässt sich die Backend-Wahl erzwingen. Beenden: `Super+m`
(Standard-Binding `exit`; mit laufender Bar danach Enter), oder per SSH `pkill -TERM mywm-compositor`.

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
   Die LEDs der Tastatur zeigen Caps/Num Lock, auch nach VT-Wechsel und Ab-/Anstecken.
3. Fenster starten (`Super+Return`), Workspaces `Super+1..9`, Monitorwechsel
   `Super+Alt+Pfeil`, Fenster verschieben `Super+Shift+Pfeil`.
4. VT-Wechsel `Ctrl+Alt+F2` und zurück: Bild und Eingabe müssen wiederkommen.
5. Monitor aus-/einstecken: Workspaces wandern zum ersten Monitor und zurück.
6. Ein Vollbild-Spiel/Video: läuft flüssig, kein Tearing.

## Spiele und VRR (M3)

mywm flippt immer synchron (kein Tearing): Rendert ein Spiel schneller als der Monitor,
zeigt jeder Refresh das neueste fertige Bild.

```toml
game_app_id_prefixes = ["steam_app_", "gamescope"]   # erkennt Spiele (auch deren Dialoge)

[vrr]
enabled = true
output = "DP-3"
```

Im Log erscheint beim Start des Spiels `DP-3: adaptive sync on`.
Beim GPU-Start steht `explicit sync (linux-drm-syncobj) available` oder der Grund, warum nicht.

Prüfen:

1. Spiel starten (Proton/Vulkan, `PROTON_ENABLE_WAYLAND=1` oder über XWayland – siehe unten):
   Vollbild deckt den Monitor komplett, keine Rahmen. Direct Scanout erkennt man daran, dass
   die GPU-Last im Vollbild kaum über den Wert ohne Compositor liegt; im Log mit
   `RUST_LOG=smithay::backend::drm=debug` erscheinen Zeilen zur Plane-Zuweisung.
2. VRR: Monitor-OSD bzw. `cat /sys/kernel/debug/dri/*/vrr_range` / Anzeige der Bildrate.
3. Maus in Spielen: Mauszeiger sperrt sich (`pointer-constraints`) und liefert rohe Deltas
   (`relative-pointer`); Spiele unter XWayland brauchen dafür M4.

## Desktop-Integration (M4)

- **Bar/Launcher**: Quickshell (`mywm-shell`) über `wlr-layer-shell`; die Bar spricht über
  `$MYWM_SOCKET` mit dem Compositor. Ohne gesetztes `MYWM_SOCKET` in der Sitzung bleibt sie leer.
- **Sperre/Idle**: `Super+Escape` startet den Sperrbildschirm (`lock.qml` aus mywm-shell, Passwort über
  den PAM-Dienst `mywm-lock`). Stirbt er, startet der Compositor einen neuen, der die Sperre übernimmt.
  Auf Hardware startet der Compositor
  außerdem `swayidle` (Sperre nach `[idle] lock_after_seconds`, danach Monitore aus über
  `wlopm`). Dafür müssen `quickshell`, `swayidle` und `wlopm` installiert sein.
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
und die Frame-Callbacks gehen am Vblank raus. Aktiv nur bei festem Refresh (nicht bei VRR).
Hilfreich: zuerst die `perf`-Zeilen ansehen; ist `worst` nahe am Refresh-Intervall, `margin_ms`
erhöhen oder die Option auslassen. Bitte Eindruck (Latenz/Ruckler) und die `perf`-Zeilen zurückmelden.

## fifo und commit-timing

mywm bietet `wp-fifo-v1` und `wp-commit-timing-v1` an. Der Compositor gibt die Sperren mit den
Frames frei: sichtbare Oberflächen mit jedem Bild ihres Ausgangs, unsichtbare (verdeckt, versteckter
Workspace, Monitor aus) höchstens einmal pro Sekunde. Ein Commit mit Zeitstempel wird frühestens ein
Refresh-Intervall vor seinem Termin freigegeben; ist der Ausgang sonst ruhig, plant mywm dafür einen
Frame ein. Nested lässt sich nur die Logik prüfen (`tests/pacing_smoke.py`), nicht das Timing.

Prüfen:

1. `wayland-info | grep -E 'wp_fifo_manager_v1|wp_commit_timing_manager_v1'` listet beide.
2. `vkcube --wsi wayland --present_mode 2` (FIFO) läuft gleichmäßig mit der Bildwiederholrate;
   mit `WAYLAND_DEBUG=client` erscheinen `wp_fifo_v1`-Aufrufe, sofern der Vulkan-Treiber fifo-v1 nutzt.
3. Natives Spiel (Proton mit `PROTON_ENABLE_WAYLAND=1`, Vsync an): läuft flüssig, keine Hänger.
   Mit `[vrr]` zusätzlich prüfen, dass VRR weiter greift.
4. Den Workspace des Spiels für ~10 s wegschalten, Monitor per `wlopm --off` aus- und wieder
   einschalten, Bildschirm sperren und entsperren: das Spiel läuft danach normal weiter und ist
   nie eingefroren (im Hintergrund darf es langsam laufen).
5. Ein SDL3-Spiel ohne `SDL_VIDEODRIVER=wayland` starten: nutzt es jetzt von selbst Wayland
   (`WAYLAND_DEBUG=client` zeigt seine Fenster), kann die Variable weg.
6. `perf`-Zeilen (siehe oben) mit und ohne Spiel vergleichen: keine zusätzlichen langsamen Frames.

## Leere Frames, gezieltes Neuzeichnen und Cursor-Plane

Ein Frame ohne Änderung bekommt kein Vblank; ein Timer steht dafür ein. Er läuft jetzt bis zum
nächsten Vblank im Raster des Ausgangs statt ein volles Intervall ab jetzt, denn ein Spiel-Frame, der
in dieser Zeit kommt, wartet mit. Leere Frames entstehen seltener: Popups, Tooltips und Menüs zeichnen
nur die Ausgänge ihres Fensters (bzw. ihrer Leiste) neu, ein neues Cursorbild nur den Ausgang des
Cursors, und solange ein Spiel den Cursor versteckt, löst Mausbewegung gar kein Neuzeichnen aus. Auf
gedrehten Ausgängen (DP-1) nutzt der Cursor die Cursor-Plane (smithays `renderer_pixman`), statt bei
jeder Bewegung zusammengesetzt zu werden.

Prüfen:

1. Spiel im Vollbild auf DP-3 mit VRR, nebenher auf einem anderen Monitor ein Programm mit Tooltips
   oder Menüs (Discord, Browser) bedienen: das Spiel läuft gleichmäßig, `perf`-Zeilen ohne neue
   langsame Frames; subjektiv keine zusätzliche Eingabeverzögerung.
2. Ein Spiel, das den Cursor versteckt, ohne ihn zu sperren (viele Menüs, manche Strategiespiele):
   Mausbewegung ruckelt nicht; beim Verlassen des Spielfensters erscheint der Cursor sofort wieder.
3. Cursor auf DP-1: erscheint richtig gedreht und mit Hotspot an der richtigen Stelle (Klick auf
   kleine Ziele, Textauswahl), wechselt seine Form (Text, Hand, Größenänderung) sofort, und die
   Bewegung dort erzeugt keine Last (`perf`-Zeilen von DP-1 zählen bei reiner Mausbewegung kaum Frames).
4. Rechtsklick-Menüs und Tooltips, auch an Monitorgrenzen und in der Bar (Tray-Menüs): erscheinen
   und verschwinden ohne Reste.

## Aufnahmen gedrehter Monitore

Aufnahmen eines Monitors (Streams über das Portal, `wlr-screencopy`, Screenshots, Farbwähler) zeigen
ihn aufrecht in seiner angezeigten Größe. Bisher galt auf der Hardware während des Zeichnens die
ungedrehte Größe (2560×1440 statt 1440×2560): Streams von DP-1 waren quer, abgeschnitten und daneben
leer, und die Übersicht dort endete mit Abdunklung und Kacheln auf 1440 Pixeln Höhe. Nested lässt sich
das mit einem gedrehten virtuellen Ausgang nachstellen (Smoke-Test `rotated`); auf DP-1 bleibt es zu
bestätigen.

Prüfen:

1. DP-1 (hochkant) streamen (Vesktop/OBS, in der Übersicht die Kachel daneben anklicken): das Bild ist
   1440×2560, aufrecht, vollständig, ohne leere Fläche; Fenster und Cursor stehen an der richtigen Stelle.
2. Dasselbe für DP-3 und HDMI-A-1 (ungedreht): unverändert richtig.
3. Übersicht auf DP-1: die Abdunklung reicht bis unten, die Kacheln sind ganz und hochkant.
4. Screenshot einer Auswahl auf DP-1 und der Farbwähler dort: zeigen bzw. treffen, was unter der
   Auswahl bzw. dem Zeiger liegt.
5. Mit `[effects] blur`: ein durchscheinendes Fenster im unteren Teil von DP-1 zeigt den unscharfen
   Hintergrund wie im oberen Teil.

## Bekannte Lücken

- Fensteraufnahme über das Portal (nur Monitore) und der Wallpaper-Picker fehlen noch.
- Der Cursor kommt aus dem xcursor-Theme (`XCURSOR_THEME`, `XCURSOR_SIZE`), animierte Cursor
  stehen still.

## Logs

Der Compositor schreibt jetzt immer eine Logdatei: `~/.local/state/mywm/compositor.log`
(Pfad mit `MYWM_LOG_FILE` änderbar, `MYWM_LOG_FILE=off` schaltet sie ab; die Datei der vorigen
Sitzung bleibt als `compositor.log.1` liegen). Darin stehen Start, Config-Pfad, jeder Monitor mit
Position und Größe, GPU und Modus, neue Fenster mit App-ID, Layer-Surfaces (Bar, Wallpaper),
Xwayland, VRR und jeder Client, der wegen eines Protokollfehlers rausgeworfen wurde.
Mit `RUST_LOG=debug` (oder z. B. `RUST_LOG=info,smithay::backend::drm=debug`) wird es
ausführlicher. Panics landen ebenfalls dort.

## Fehler melden

Bitte `compositor.log` (mit `RUST_LOG=debug` für Ausgaben von `smithay`) und die Ausgabe von
`dmesg | grep -i -E "nvidia|drm"` mitschicken.
