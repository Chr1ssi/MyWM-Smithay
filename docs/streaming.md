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

- **Nur Monitor-Aufnahme.** Einzelne Fenster aufnehmen (OBS „Fensteraufnahme“ über das Portal)
  setzt `ext-image-copy-capture` mit Toplevel-Quellen voraus; das gibt es hier noch nicht.
  Workaround: Monitor aufnehmen und in OBS zuschneiden, oder ein eigenes Spielfenster im
  Vollbild auf einem Monitor laufen lassen.
- Skalierte oder gedrehte Monitore werden so aufgenommen, wie sie dargestellt werden (in
  physischen Pixeln, nach der Drehung).
- Mit `copy_with_damage` (OBS, wf-recorder) wird nur bei tatsächlichen Änderungen ein neues Bild
  geliefert; bei ruhigem Bild sinkt die Rate entsprechend.
