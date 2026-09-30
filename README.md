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
Bildschirmaufnahme (`wlr-screencopy` → OBS, Browser, grim). M2–M4 wurden nested
getestet (Layer-Shell, Sperre, Idle, Monitor-Power, Xwayland, Capture, Multi-Monitor
mit virtuellen Ausgängen); der Hardware-Teil (DRM, libinput, VRR, Tearing, Scanout)
lief noch nicht auf echter Hardware, siehe `docs/hardware-test.md`.

Langfristig läuft alles nativ unter Wayland: Xwayland ist nur für Apps da, die es noch
brauchen (Steam selbst, ältere Spiele) und lässt sich mit `xwayland = false` abschalten.
Spiele laufen nativ (SDL: `SDL_VIDEODRIVER=wayland`, Proton: `PROTON_ENABLE_WAYLAND=1`).

Aufbau:

- `crates/mywm-layout` – Scrolling, Workspaces, Geometrie (aus dem River-MyWM
  portiert, ohne Compositor-Abhängigkeit, mit Tests)
- `crates/mywm-config` – TOML-Konfiguration, **gleiches Schema wie das
  River-MyWM** (`~/.config/mywm/config.toml` oder `$MYWM_CONFIG`); neu sind nur
  `toggle_fullscreen`, `column_shrink`, `column_grow`. Beispiel:
  `crates/mywm-config/example.toml`
- `crates/mywm-ipc` – Bar-Protokoll `v1` (Befehle, Zustands-Snapshot), unverändert
  gegenüber dem River-MyWM, damit `mywm-shell` ohne Änderung läuft
- `vendor/smithay` – smithay 0.7.0 mit kleinen Patches (Tearing, Layer-Shell-Toleranz, X11-Fokus; `vendor/README.md`)
- `src/` – Compositor; `udev.rs` ist das Hardware-Backend (DRM/GBM/libinput/libseat),
  `winit.rs` der nested Entwicklungsmodus (`desktop.rs` bildet das Modell auf Smithays `Space` ab)

Roadmap: M5 Profiling und Feinschliff (späte Frame-Planung, Frame-Pacing), danach
Fensteraufnahme (`ext-image-copy-capture`), Wallpaper-Picker/Theme-Generierung und
`wlr-output-management`. Noch nicht portiert: Wallpaper-Picker und Theme-Generierung
(Binding `wallpaper` tut nichts).

## Entwickeln

Build-Abhängigkeiten (Debian/Ubuntu): `libwayland-dev libxkbcommon-dev libinput-dev
libseat-dev libgbm-dev libdrm-dev libudev-dev libegl-dev libgles-dev`.

```sh
cargo run -- "kitty"      # nested (in X/Wayland); optionales Kommando nach dem Start
cargo test --workspace    # Layout, Config, Protokoll
PYTHONPATH=tests python3 tests/ipc_smoke.py           # Bar-Socket (braucht X + weston-simple-shm)
PYTHONPATH=tests python3 tests/multimonitor_smoke.py  # Multi-Monitor mit virtuellen Ausgängen
PYTHONPATH=tests python3 tests/globals_smoke.py       # angebotene Wayland-Protokolle
PYTHONPATH=tests python3 tests/session_smoke.py       # Sperre, Idle, Monitor-Power (swaylock/swayidle/wlopm)
PYTHONPATH=tests python3 tests/xwayland_smoke.py      # X11-Clients (xterm, xeyes)
PYTHONPATH=tests python3 tests/screencopy_smoke.py    # Aufnahme mit grim
PYTHONPATH=tests python3 tests/clipboard_smoke.py     # Kopieren/Einfügen (wl-clipboard)
PYTHONPATH=tests python3 tests/layer_smoke.py         # Layer-Shell-Client mit Größe 0 (Quickshell-Marker)
PYTHONPATH=tests python3 tests/x11_focus_smoke.py     # Tastatur in X11-Fenstern (xev)
PYTHONPATH=tests python3 tests/scroll_clip_smoke.py   # herausgescrollte Fenster erscheinen nicht auf dem Nachbarmonitor
```

Im nested Betrieb gehört Super dem Host: Alle `Super`-Bindings gelten dort als
**Alt** (Bindings mit `Super+Alt` entfallen). `MYWM_MODKEY=super` schaltet das ab,
etwa unter einem Host ohne eigene Super-Belegung. Standard-Bindings: siehe
`example.toml`; die Standardtastatur ist `de`.
