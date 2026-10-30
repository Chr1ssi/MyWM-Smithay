# Effekte, Screenshots und Übersicht

Alle Effekte sind standardmäßig **aus** und gelten nie für Vollbildfenster, also auch nicht
für Spiele: Direct Scanout und Latenz bleiben unberührt. Einstellungen in `config.toml`
(`Super+Shift+r` lädt neu):

```toml
[effects]
corner_radius = 12       # abgerundete Fenster und Rahmen (logische Pixel, 0-64)
window_opacity = 0.94    # alle Fenster leicht durchsichtig (nie Videofenster)
inactive_opacity = 0.95  # Fenster ohne Fokus (nie Videofenster)
animation_ms = 160       # Fenster gleiten an ihren Platz und blenden ein (0 = aus)
shadow = 24              # weicher Schatten
blur = 6                 # unscharfes Wallpaper hinter durchsichtigen Fenstern (1-16)

[[rules]]                # Transparenz pro Programm (app_id siehe Log: „new window: app_id=…“)
app_id = "kitty"
opacity = 0.92
```

- **Videofenster** bleiben undurchsichtig, auch ohne Vollbild, und bekommen weder `window_opacity` noch
  `inactive_opacity` noch Blur. Ein Fenster gilt als Video, wenn eine Regel `video = true` setzt (`video = false`
  nimmt es aus), sonst wenn der Client seiner Fläche oder einer Unterfläche den Inhaltstyp `video`
  (`wp-content-type`) gibt oder einen Idle-Inhibitor auf dem Fenster hält (mpv, VLC, solange sie spielen). Browser
  sperren den Bildschirm über das Portal, das kein Fenster nennt: Für sie braucht es eine Regel
  (`app_id = "librewolf"`, `video = true`), dann ist das ganze Browserfenster undurchsichtig.
- Eine Regel mit `opacity` **ersetzt** `window_opacity` für ihr Fenster (`opacity = 1.0` nimmt es aus); die
  Deckkraft ohne Fokus wird darauf multipliziert.

- **Blur** ist „Xray“: Durchsichtige Fenster zeigen eine weichgezeichnete Kopie des
  Wallpapers (samt Layern unter den Fenstern), nicht der Fenster dahinter. Die Kopie wird nur neu
  berechnet, wenn sich das Wallpaper ändert. Ein Fenster ist durchsichtig, wenn seine Deckkraft
  (`window_opacity`, Regel, `inactive_opacity` ohne Fokus) unter 1 liegt und es kein Video ist.
- **Wallpaper-Wechsel**: Mit `animation_ms` > 0 blendet ein neues Wallpaper in 600 ms über das alte (ease-out).
  Dasselbe Bild für eine andere Monitorgröße ersetzt das alte ohne Überblendung. Wechselt man mitten in der
  Überblendung, verschwindet das auslaufende Bild sofort. Der Blur zeigt während der Überblendung noch das
  alte Wallpaper und springt danach um.
- Die Shader wurden nested (Mesa) getestet; auf der NVIDIA-Hardware bitte kurz ansehen. Kompilieren
  sie nicht oder schlägt die Berechnung fehl, steht einmal eine Warnung im Log und der Effekt bleibt
  bis zum nächsten Neuladen der Konfiguration aus.

## Screenshots

| Taste | Wirkung |
|---|---|
| `Super+s` | Bereich ziehen oder Fenster anklicken (Enter = ganzer Monitor, Esc/Rechtsklick = abbrechen) |
| `Super+Shift+s` | Monitor unter dem Mauszeiger |
| `Super+Ctrl+s` | fokussiertes Fenster (mit Rahmen) |

Farbpipette (`color_picker`, ohne Standardtaste): Neben dem Mauszeiger zeigt eine Lupe die Pixel
darum, das mittlere markiert; die Bar zeigt dessen Hex-Wert. Ein Klick (oder Enter) kopiert ihn als
`#rrggbb` in die Zwischenablage und meldet ihn mit einem Farbfeld, Esc oder eine andere Maustaste
bricht ab. Die Farbe kommt wie bei Screenshots aus dem fertigen Bild, ohne Cursor und Lupe.

Das PNG landet in `screenshot_directory` (Standard `~/Bilder/Screenshots`, Name
`Screenshot_JJJJ-MM-TT_HH-MM-SS.png`) und in der Zwischenablage (braucht `wl-copy`); mit
`notify-send` gibt es eine Benachrichtigung. Cursor und Auswahl-Overlay sind nie im Bild.

## Workspace-Übersicht

`Super+Tab` zeigt auf **jedem Monitor** alle seine Workspaces als Vorschau mit den echten Fenstern,
dahinter eine Kachel `+` für einen neuen Workspace und eine für das Scratchpad (grau umrandet).
Beim Öffnen zoomt die Übersicht aus dem gezeigten Workspace heraus, beim Schließen wieder hinein
(Dauer `animation_ms`, ohne Animation sofort). Tasten und Klicks wirken auf den Monitor unter dem
Mauszeiger; dort ist die gewählte Kachel weiß umrandet.

| Eingabe | Wirkung |
|---|---|
| Klick auf eine Kachel, Pfeiltasten + Enter | dorthin wechseln (`+`: neuer Workspace, Scratchpad: einblenden) |
| `1` … `9` | direkt zu diesem Workspace, egal auf welchem Monitor |
| Klick auf ein Fenster | Fenster zeigen und fokussieren |
| Fenster ziehen | auf einen anderen Workspace verschieben, auch auf einen anderen Monitor; auf `+` in einen neuen Workspace, auf das Scratchpad dorthin. Die Übersicht bleibt offen. |
| `q` oder Mittelklick auf ein Fenster | Fenster schließen |
| Esc, Klick neben die Kacheln | schließen |

Kachelrahmen: weiß die gewählte, rot eine mit einem Fenster, das Aufmerksamkeit will
(`urgent_border`), in der Akzentfarbe der gerade gezeigte Workspace. Die Tasten ändert man in
`[bindings]` (`overview`, `screenshot`, `screenshot_screen`, `screenshot_window`).

Standard-Tastenbelegungen rechnen mit einer 80-%-Tastatur (ohne Nummernblock und ohne
Druck-Taste); alles lässt sich in `[bindings]` ändern.
