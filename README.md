# mywm-compositor

Smithay-basierter Wayland-Compositor für [MyWM](https://github.com/Chr1ssi/mywm):
leicht, latenzarm (Gaming) und mit sauberem Screen-Capture (Streaming).

## Status

**M0** (Skelett) und **M1** (Layout-Kern) sind fertig: nested Betrieb über winit,
xdg-shell, Fokus folgt der Maus, horizontales Scrolling-Layout mit Gaps und
Fokusrahmen, Workspaces (1–9, dynamisch, Wechsel/Verschieben), Floating mit
Mausverschieben/-skalieren, Vollbild, Dialoge folgen ihrem Elternfenster.

Die Logik liegt in `crates/mywm-layout` (aus dem River-MyWM portiert, ohne
Compositor-Abhängigkeit, mit Unit-Tests). `src/desktop.rs` bildet sie auf
Smithays `Space` ab.

Roadmap: M2 DRM/libinput + Multi-Monitor, M3 Direct Scanout / Tearing / VRR,
M4 XWayland / Layer-Shell / Screencast, M5 Profiling. Offen aus MyWM:
TOML-Konfiguration, Regeln, Scratchpad, Gaming-Workspace.

## Entwickeln

Build-Abhängigkeiten (Debian/Ubuntu): `libwayland-dev libxkbcommon-dev libinput-dev
libseat-dev libgbm-dev libdrm-dev libudev-dev libegl-dev libgles-dev`.

```sh
cargo run -- "kitty"      # optionales Kommando wird nach dem Start ausgeführt
cargo test --workspace    # Layout-Logik
```

Im nested Betrieb dient **Alt** als Modifier (Super gehört dem Host):

| Taste | Aktion |
| --- | --- |
| `Alt+Return` | Terminal (`$MYWM_TERMINAL`, Standard `kitty`) |
| `Alt+Q` | Fokussiertes Fenster schließen |
| `Alt+H/L` (oder Pfeile) | Fokus links/rechts |
| `Alt+Shift+H/L` | Spalte verschieben |
| `Alt+-` / `Alt+=` | Spaltenbreite ändern |
| `Alt+1..9`, `Alt+Shift+1..9` | Workspace wechseln / Fenster verschieben |
| `Alt+Ctrl+H/L` | Workspace zyklisch wechseln |
| `Alt+N`, `Alt+Shift+N` | Neuer Workspace / Fenster in neuen Workspace |
| `Alt+V` | Floating umschalten |
| `Alt+F` | Vollbild umschalten |
| `Alt`+linke Maustaste ziehen | Floating-Fenster verschieben |
| `Alt`+rechte Maustaste ziehen | Floating-Fenster oder Spalte skalieren |
| `Alt+Shift+E` | Compositor beenden |
