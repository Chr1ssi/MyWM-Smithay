# mywm-compositor

Smithay-basierter Wayland-Compositor für [MyWM](https://github.com/Chr1ssi/mywm):
leicht, latenzarm (Gaming) und mit sauberem Screen-Capture (Streaming).

## Status

**M0** (Skelett), **M1** (Layout-Kern) und die **Konfiguration** sind fertig:
nested Betrieb über winit, xdg-shell, Fokus folgt der Maus, horizontales
Scrolling-Layout mit Gaps und Fokusrahmen, Workspaces (1–9, dynamisch), Floating
mit Mausverschieben/-skalieren, Vollbild, Dialoge folgen ihrem Elternfenster,
globaler Scratchpad, Bar-Socket (`$MYWM_SOCKET`), Fensterregeln, Gaming-Workspace (über
`game_app_id_prefixes`), Live-Reload mit `Super+Shift+r`.

Aufbau:

- `crates/mywm-layout` – Scrolling, Workspaces, Geometrie (aus dem River-MyWM
  portiert, ohne Compositor-Abhängigkeit, mit Tests)
- `crates/mywm-config` – TOML-Konfiguration, **gleiches Schema wie das
  River-MyWM** (`~/.config/mywm/config.toml` oder `$MYWM_CONFIG`); neu sind nur
  `toggle_fullscreen`, `column_shrink`, `column_grow`. Beispiel:
  `crates/mywm-config/example.toml`
- `crates/mywm-ipc` – Bar-Protokoll `v1` (Befehle, Zustands-Snapshot), unverändert
  gegenüber dem River-MyWM, damit `mywm-shell` ohne Änderung läuft
- `src/` – Compositor (`desktop.rs` bildet das Modell auf Smithays `Space` ab)

Roadmap: M2 DRM/libinput + Multi-Monitor, M3 Direct Scanout / Tearing / VRR,
M4 XWayland / Layer-Shell / Screencast, M5 Profiling. Noch nicht portiert:
Sperrbildschirm/Idle, Wallpaper-Picker und Theme-Generierung,
`workspace_outputs`/`gaming_output` (brauchen Multi-Monitor).
Die Bindings `lock`, `wallpaper` und `*_output_*` werden geparst, tun aber noch nichts.

## Entwickeln

Build-Abhängigkeiten (Debian/Ubuntu): `libwayland-dev libxkbcommon-dev libinput-dev
libseat-dev libgbm-dev libdrm-dev libudev-dev libegl-dev libgles-dev`.

```sh
cargo run -- "kitty"      # optionales Kommando wird nach dem Start ausgeführt
cargo test --workspace    # Layout, Config, Protokoll
python3 tests/ipc_smoke.py  # Bar-Socket gegen den laufenden Compositor (braucht X + weston-simple-shm)
```

Im nested Betrieb gehört Super dem Host: Alle `Super`-Bindings gelten dort als
**Alt** (Bindings mit `Super+Alt` entfallen). `MYWM_MODKEY=super` schaltet das ab,
etwa unter einem Host ohne eigene Super-Belegung. Standard-Bindings: siehe
`example.toml`; die Standardtastatur ist `de`.
