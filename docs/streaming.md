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
