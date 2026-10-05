# Effekte, Screenshots und Übersicht

Alle Effekte sind standardmäßig **aus** und gelten nie für Vollbildfenster, also auch nicht
für Spiele: Direct Scanout und Latenz bleiben unberührt. Einstellungen in `config.toml`
(`Super+Shift+r` lädt neu):

```toml
[effects]
corner_radius = 12       # abgerundete Fenster und Rahmen (logische Pixel, 0-64)
inactive_opacity = 0.95  # Fenster ohne Fokus
animation_ms = 160       # Fenster gleiten an ihren Platz und blenden ein (0 = aus)
shadow = 24              # weicher Schatten
blur = 6                 # unscharfes Wallpaper hinter durchsichtigen Fenstern (1-16)

[[rules]]                # Transparenz pro Programm (app_id siehe Log: „new window: app_id=…“)
app_id = "kitty"
opacity = 0.92
```

- **Blur** ist „Xray“: Durchsichtige Fenster zeigen eine weichgezeichnete Kopie des
  Wallpapers (samt Layern unter den Fenstern), nicht der Fenster dahinter. Die Kopie wird nur neu
  berechnet, wenn sich das Wallpaper ändert. Ein Fenster ist durchsichtig, wenn eine Regel
  `opacity` setzt oder `inactive_opacity` < 1 ist und es keinen Fokus hat.
- Die Shader wurden nested (Mesa) getestet; auf der NVIDIA-Hardware bitte kurz ansehen. Kompilieren
  sie nicht, steht eine Warnung im Log und der Effekt bleibt einfach aus.

## Screenshots

| Taste | Wirkung |
|---|---|
| `Super+s` | Bereich ziehen oder Fenster anklicken (Enter = ganzer Monitor, Esc/Rechtsklick = abbrechen) |
| `Super+Shift+s` | Monitor unter dem Mauszeiger |
| `Super+Ctrl+s` | fokussiertes Fenster (mit Rahmen) |

Das PNG landet in `screenshot_directory` (Standard `~/Bilder/Screenshots`, Name
`Screenshot_JJJJ-MM-TT_HH-MM-SS.png`) und in der Zwischenablage (braucht `wl-copy`); mit
`notify-send` gibt es eine Benachrichtigung. Cursor und Auswahl-Overlay sind nie im Bild.

## Workspace-Übersicht

`Super+Tab` zeigt alle Workspaces des Monitors unter dem Mauszeiger als Vorschau mit den echten
Fenstern. Klick auf eine Vorschau (oder Pfeiltasten + Enter) wechselt dorthin, Esc oder ein Klick
daneben schließt. Die Tasten ändert man in `[bindings]` (`overview`, `screenshot`,
`screenshot_screen`, `screenshot_window`).

Standard-Tastenbelegungen rechnen mit einer 80-%-Tastatur (ohne Nummernblock und ohne
Druck-Taste); alles lässt sich in `[bindings]` ändern.
