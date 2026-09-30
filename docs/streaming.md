# Streaming und Bildschirmaufnahme

Der Compositor bietet `wlr-screencopy-unstable-v1` an, das Protokoll, das
`xdg-desktop-portal-wlr` benutzt. Damit funktionieren:

- **OBS** (Quelle „Bildschirmaufnahme (PipeWire)“), **Discord**, **Browser** (Bildschirm teilen),
  über Portal und PipeWire;
- **grim**, `wf-recorder` und ähnliche Werkzeuge direkt.

GPU-Puffer (dmabuf) werden direkt beschrieben, ohne Umweg über die CPU; Shared-Memory-Puffer
brauchen einmal Auslesen der GPU. Auf der Hardware verwendet der Portal-Dienst automatisch den
dmabuf-Weg, wenn OBS/der Browser ihn versteht.

## Einrichtung (wie beim River-MyWM)

Die Portal-Konfiguration aus dem MyWM-Repo (`config/river-portals.conf`,
`config/river-screencast.conf`) gilt unverändert weiter: der Compositor setzt auf Hardware
`XDG_CURRENT_DESKTOP=river`, falls es nicht gesetzt ist, damit `xdg-desktop-portal-wlr`
(`wlr.portal` führt `river` unter `UseIn`) sich angesprochen fühlt. Eigene Werte in der
Sitzungsumgebung haben Vorrang.

Nötige Pakete: `xdg-desktop-portal`, `xdg-desktop-portal-wlr`, `pipewire`, `wireplumber`.
Die Portale müssen nach dem Start des Compositors die Umgebung kennen
(`WAYLAND_DISPLAY`, `XDG_CURRENT_DESKTOP`), wie es `scripts/session-environment` schon tut.

## Grenzen

- **Fensteraufnahme** gibt es über `ext-foreign-toplevel-list` + `ext-image-copy-capture`
  (Toplevel-Quellen, SHM und Dmabuf). `xdg-desktop-portal-wlr` spricht nur `wlr-screencopy`
  und damit nur Monitore; für Fenster braucht es ein Portal oder Programm mit Unterstützung
  dieser Protokolle. Bis dahin: Monitor aufnehmen und in OBS zuschneiden. Der Cursor wird nur
  mit `paint_cursors` ins Bild gemalt, eigene Cursor-Sessions liefern nichts.
- Skalierte oder gedrehte Monitore werden so aufgenommen, wie sie dargestellt werden (in
  physischen Pixeln, nach der Drehung).
- Mit `copy_with_damage` (OBS, wf-recorder) wird nur bei tatsächlichen Änderungen ein neues Bild
  geliefert; bei ruhigem Bild sinkt die Rate entsprechend.

## Schnelltest ohne OBS: Vesktop/Discord

1. Prüfen, dass die Portale laufen: `systemctl --user status xdg-desktop-portal xdg-desktop-portal-wlr pipewire`.
2. In Vesktop: Sprachkanal beitreten → „Bildschirm teilen“. Das geht über Portal und PipeWire, es
   erscheint die Monitor-Auswahl des Portals (`chooser_type` in `xdg-desktop-portal-wlr`, siehe
   `config/river-screencast.conf`). Nur **Monitore** stehen zur Wahl, keine Fenster.
3. Erst ohne Portal prüfen, ob der Compositor liefert: `grim /tmp/test.png` muss ein Bild erzeugen.
   Geht das, liegt ein Fehler bei Portal/PipeWire, nicht beim Compositor.
4. Ton beim Teilen übernimmt Vesktop selbst (Venmic), nicht das Portal.

Ohne Discord geht es auch im Browser mit der Seite „getDisplayMedia“ von webrtc.github.io/samples.
