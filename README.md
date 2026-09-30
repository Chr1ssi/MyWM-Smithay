# mywm-compositor

Smithay-basierter Wayland-Compositor für [MyWM](https://github.com/Chr1ssi/mywm):
leicht, latenzarm (Gaming) und mit sauberem Screen-Capture (Streaming).

## Status

**M0** – Skelett: nested Betrieb über das winit-Backend, xdg-shell, wl_shm,
Seat (Tastatur/Zeiger), Fokus folgt der Maus, Platzhalter-Layout (gleich breite
Spalten). Roadmap: M1 Layout-Kern portieren, M2 DRM/libinput, M3 Direct Scanout /
Tearing / VRR, M4 XWayland / Layer-Shell / Screencast, M5 Profiling.

## Entwickeln

Build-Abhängigkeiten (Debian/Ubuntu): `libwayland-dev libxkbcommon-dev libinput-dev
libseat-dev libgbm-dev libdrm-dev libudev-dev libegl-dev libgles-dev`.

```sh
cargo run -- "kitty"      # optionales Kommando wird nach dem Start ausgeführt
```

Im nested Betrieb dient **Alt** als Modifier (Super gehört dem Host):

| Taste | Aktion |
| --- | --- |
| `Alt+Return` | Terminal (`$MYWM_TERMINAL`, Standard `kitty`) |
| `Alt+Q` | Fokussiertes Fenster schließen |
| `Alt+Shift+E` | Compositor beenden |
