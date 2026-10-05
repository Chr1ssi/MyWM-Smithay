# mywm-compositor

Smithay-basierter Wayland-Compositor für [MyWM](https://github.com/Chr1ssi/mywm):
leicht, latenzarm (Gaming) und mit sauberem Screen-Capture (Streaming).

## Status

**M0–M4** sind fertig: Skelett, Layout-Kern, Konfiguration, Multi-Monitor mit
DRM/libinput, Gaming-Pfad (Direct Scanout mit Dmabuf-Feedback, VRR, Tearing,
Presentation-Time, explizite Sync, Relative Pointer/Pointer Constraints, Viewporter,
Fractional Scale) und Desktop-Integration: Layer-Shell (Quickshell-Bar, Launcher),
Sitzungssperre (`ext-session-lock`/swaylock), Idle (`ext-idle-notify`/swayidle,
`wlr-output-power-management`/wlopm), Xwayland für Legacy-Apps (Steam) und
Bildschirmaufnahme (`wlr-screencopy` → OBS, Browser, grim). Der Compositor läuft im
Alltag auf echter Hardware (NixOS, eigene Sitzung); die nested Smoke-Tests decken Layer-Shell,
Sperre, Idle, Monitor-Power, Xwayland, Capture und Multi-Monitor mit virtuellen Ausgängen ab.
Hinweise zum Test auf neuer Hardware: `docs/hardware-test.md`.

Langfristig läuft alles nativ unter Wayland: Xwayland ist nur für Apps da, die es noch
brauchen (Steam selbst, ältere Spiele) und lässt sich mit `xwayland = false` abschalten.
Spiele laufen nativ (SDL: `SDL_VIDEODRIVER=wayland`, Proton: `PROTON_ENABLE_WAYLAND=1`).

Aufbau:

- `crates/mywm-layout` – Scrolling, Workspaces, Geometrie (ohne Compositor-Abhängigkeit, mit Tests)
- `crates/mywm-config` – TOML-Konfiguration (`~/.config/mywm/config.toml` oder `$MYWM_CONFIG`).
  Beispiel: `crates/mywm-config/example.toml`
- `crates/mywm-ipc` – Bar-Protokoll `v1` (Befehle, Zustands-Snapshot) zwischen Compositor und `mywm-shell`
- `vendor/smithay` – smithay 0.7.0 mit kleinen Patches (Tearing, Layer-Shell-Toleranz, X11-Fokus; `vendor/README.md`)
- `src/` – Compositor; `udev.rs` ist das Hardware-Backend (DRM/GBM/libinput/libseat),
  `winit.rs` der nested Entwicklungsmodus (`desktop.rs` bildet das Modell auf Smithays `Space` ab)

**M5** (Frame-Pacing) ist umgesetzt: Redraws pro Ausgang statt für alle, Messung der
Renderzeit (`RUST_LOG=info,perf=debug`), Direct-Scanout-Log und optionale späte
Frame-Planung (`[render] late_scheduling`, standardmäßig aus, bis sie auf Hardware
verglichen wurde).

Außerdem fertig: Fensteraufnahme (`ext-foreign-toplevel-list`, `ext-image-copy-capture`;
sie braucht ein Portal/Programm, das diese Protokolle spricht), `wlr-output-management`
(kanshi, wlr-randr: Modus, Position, Skalierung, Drehung; Ausgänge abschalten geht nicht),
Wallpaper-Picker und Theme-Generierung aus dem Wallpaper (Crate `mywm-theme`, Helfer-Modi
`--wallpaper`, `--wallpaper-list`, `--theme-from-wallpaper`, `--theme-from-state`, `--bar`,
`--launcher` der Binary; `--wallpaper` muss in der Sitzung laufen, das Binding `wallpaper`
öffnet dort den Picker). Das Wallpaper selbst zeichnet der Compositor: Er liest die Auswahl aus
`$XDG_STATE_HOME/mywm/wallpaper.json` (sonst das erste Bild in `wallpaper_directory`), skaliert sie
einmal in einem eigenen Thread über alle Monitore und lädt sie beim Theme-Reload des Pickers neu. Der
Quickshell-Picker (`mywm-shell`) sieht aus wie bisher, zeichnet aber keinen eigenen Hintergrund mehr.

Außerdem: Effekte (abgerundete Ecken, Transparenz, Schatten, Animationen, Xray-Blur; siehe
`docs/effects.md`), eingebaute Screenshots (`Super+s`) und eine Workspace-Übersicht (`Super+Tab`).

Screen-Sharing ohne `xdg-desktop-portal-wlr`: `mywm-portal` fragt im Compositor nach Fenster oder Monitor
und streamt über PipeWire (`docs/streaming.md`).

Installation, Session und NixOS-Modul: `docs/install.md`. Zusätzlich: `[input]` (Beschleunigung,
Scrollrichtung, Tastenwiederholung), `keyboard-shortcuts-inhibit` (mit `Super+Shift+Escape` holt man die Kürzel
zurück), `xdg-activation` (Fenster mit Aufmerksamkeitswunsch bekommen einen roten Rahmen) und `wlr-gamma-control`
(Nachtlicht mit gammastep/wlsunset, nur Hardware).

Einstellungen und Tastenbelegung per Fenster: `mywm-settings` (`docs/settings.md`), schreibt direkt in die Config-Datei
(auch hinter einem Dotfiles-Symlink) und lässt den Compositor neu laden.

Offen: Hardware-Verifikation von Tearing, Hotplug, Late Scheduling, den neuen Protokollen, der Effekt-Shader und des Nachtlichts.

## Entwickeln

Unter NixOS liefert `nix develop` alle Build-Abhängigkeiten (plus clippy, rustfmt, rust-analyzer); `cargo` wird
darin aufgerufen. Ohne Nix (Debian/Ubuntu): `libwayland-dev libxkbcommon-dev libinput-dev
libseat-dev libgbm-dev libdrm-dev libudev-dev libegl-dev libgles-dev`; für `mywm-portal` zusätzlich
`libpipewire-0.3-dev libspa-0.2-dev clang`.

```sh
cargo run -- "kitty"      # nested (in X/Wayland); optionales Kommando nach dem Start
cargo test --workspace    # Layout, Config, Protokoll
```

Die nested Smoke-Tests (`tests/*_smoke.py`) laufen unter NixOS alle zusammen gegen ein eigenes Xvfb, das die echte
Sitzung in Ruhe lässt (sie bewegen den X-Zeiger und tippen ins Fenster):

```sh
nix develop .#smoke -c tests/run-smoke              # alle
nix develop .#smoke -c tests/run-smoke popup ipc    # einzelne (Name ohne _smoke.py)
```

Einzeln, in `nix develop .#smoke` und mit gesetztem `$DISPLAY` (z. B. Xvfb):

```sh
PYTHONPATH=tests python3 tests/ipc_smoke.py           # Bar-Socket (braucht X)
PYTHONPATH=tests python3 tests/multimonitor_smoke.py  # Multi-Monitor mit virtuellen Ausgängen
PYTHONPATH=tests python3 tests/globals_smoke.py       # angebotene Wayland-Protokolle
PYTHONPATH=tests python3 tests/session_smoke.py       # Sperre, Idle, Monitor-Power (swaylock/swayidle/wlopm)
PYTHONPATH=tests python3 tests/xwayland_smoke.py      # X11-Clients (xterm, xeyes)
PYTHONPATH=tests python3 tests/screencopy_smoke.py    # Aufnahme mit grim
PYTHONPATH=tests python3 tests/clipboard_smoke.py     # Kopieren/Einfügen (wl-clipboard)
PYTHONPATH=tests python3 tests/layer_smoke.py         # Layer-Shell-Client mit Größe 0 (Quickshell-Marker)
PYTHONPATH=tests python3 tests/x11_focus_smoke.py     # Tastatur in X11-Fenstern (xev)
PYTHONPATH=tests python3 tests/scroll_clip_smoke.py   # herausgescrollte Fenster erscheinen nicht auf dem Nachbarmonitor
PYTHONPATH=tests python3 tests/output_management_smoke.py  # wlr-randr: Position/Skalierung
PYTHONPATH=tests python3 tests/image_capture_smoke.py # Fenster-/Monitoraufnahme (crates/mywm-capture-test)
PYTHONPATH=tests python3 tests/effects_smoke.py       # runde Ecken, Transparenz, Schatten, Fade, Blur (braucht swaybg)
PYTHONPATH=tests python3 tests/screenshot_smoke.py    # eingebaute Screenshots
PYTHONPATH=tests python3 tests/overview_smoke.py      # Workspace-Übersicht
PYTHONPATH=tests python3 tests/layer_focus_smoke.py    # Launcher/Picker bekommen die Tastatur ohne Klick
PYTHONPATH=tests python3 tests/stacking_smoke.py      # Vollbild über der Bar, Rahmen unter Overlays
PYTHONPATH=tests python3 tests/attention_smoke.py     # Shortcut-Inhibit und Urgent-Rahmen (crates/mywm-test-client)
PYTHONPATH=tests python3 tests/pointer_focus_smoke.py # gesperrter Zeiger (Spiel) wird beim Workspace-Wechsel frei
PYTHONPATH=tests python3 tests/settings_smoke.py      # Einstellungs-GUI: Taste aufnehmen, speichern, Duplikate
PYTHONPATH=tests python3 tests/portal_smoke.py        # mywm-portal: D-Bus, Auswahl im Compositor, PipeWire (braucht pipewire, wireplumber, dbus-daemon, gst)
```

Im nested Betrieb gehört Super dem Host: Alle `Super`-Bindings gelten dort als
**Alt** (Bindings mit `Super+Alt` entfallen). `MYWM_MODKEY=super` schaltet das ab,
etwa unter einem Host ohne eigene Super-Belegung. Standard-Bindings: siehe
`example.toml`; die Standardtastatur ist `de`.

## Lizenz

MIT, siehe `LICENSE`. Das mitgelieferte Smithay unter `vendor/smithay` steht ebenfalls unter MIT
(`vendor/smithay/LICENSE.txt`).
