# Streaming und Bildschirmaufnahme

Der Compositor bietet `wlr-screencopy-unstable-v1` an, das Protokoll, das
`xdg-desktop-portal-wlr` benutzt. Damit funktionieren:

- **OBS** (Quelle „Bildschirmaufnahme (PipeWire)“), **Discord**, **Browser** (Bildschirm teilen),
  über Portal und PipeWire;
- **grim**, `wf-recorder` und ähnliche Werkzeuge direkt.

GPU-Puffer (dmabuf) werden direkt beschrieben, ohne Umweg über die CPU; Shared-Memory-Puffer
brauchen einmal Auslesen der GPU. Auf der Hardware verwendet der Portal-Dienst automatisch den
dmabuf-Weg, wenn OBS/der Browser ihn versteht.

## Eigenes Portal: `mywm-portal` (empfohlen)

`mywm-portal` ersetzt `xdg-desktop-portal-wlr`. Es ist das ScreenCast-Backend von
`xdg-desktop-portal` und fragt **im Compositor** nach der Quelle: Beim Teilen (Vesktop, Discord,
Browser, OBS) erscheint eine Auswahl auf dem Bildschirm. Ein Klick auf ein **Fenster** teilt dieses
Fenster, ein Klick auf den Desktop (oder Enter) teilt den **Monitor** unter dem Mauszeiger, Esc
bricht ab. Anders als beim wlr-Portal funktionieren damit auch einzelne Fenster, und es läuft
ohne `slurp`/Chooser-Programm. Die Fenstergröße wird live nachgeführt, der Cursor kann mitgemalt werden.

Einrichtung:

1. `cargo build --release -p mywm-portal` und die Binary `mywm-portal` in den `PATH` legen.
2. Aus `portal/` installieren: `mywm.portal` nach `…/share/xdg-desktop-portal/portals/`,
   `org.freedesktop.impl.portal.desktop.mywm.service` nach `…/share/dbus-1/services/`,
   `mywm-portal.service` nach `~/.config/systemd/user/` (die D-Bus-Aktivierung startet das Portal als
   diese Unit, die mit der Sitzung endet; ohne sie bliebe ein Portal mit dem Socket einer alten
   Sitzung hängen), und die
   Portal-Auswahl `mywm-portals.conf` nach `~/.config/xdg-desktop-portal/` (Sitzung und Compositor
   setzen `XDG_CURRENT_DESKTOP=mywm`).
   Unter NixOS gehört `mywm.portal` in ein Paket, das unter `xdg.portal.extraPortals` steht.
3. Das Portal braucht in seiner Umgebung `WAYLAND_DISPLAY` und `MYWM_SOCKET` (derselbe Socket wie
   für die Bar, sonst `$XDG_RUNTIME_DIR/mywm.sock`). Die Sitzung sollte sie dem D-Bus bekanntmachen:
   `dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP MYWM_SOCKET`.
4. `xdg-desktop-portal-wlr` nicht mehr starten. Nötig bleiben `xdg-desktop-portal`, `pipewire`, `wireplumber`.

Ruhige Bilder: Steht das Bild still, liefert der Compositor trotzdem etwa alle 0,4 s ein Bild (Keepalive),
sonst hält Chromium (Discord, Vesktop) den Strom für tot. Öffnet eine App zwei Sitzungen gleichzeitig (Vesktop tut das),
beantwortet die Auswahl beide; die zweite wird nicht abgelehnt.

Logs: `journalctl --user -u mywm-portal` bzw. die Ausgabe von `mywm-portal` zeigt Format, Bildgröße,
Verbraucher und Bilder pro 5 s; im Compositor-Log stehen `capture session: …` (Quelle, Größe, erstes Bild, Ende).

Technik: Das Portal hängt als Wayland-Client am Compositor (`ext-image-copy-capture`) und schreibt
die Bilder direkt in PipeWire-Puffer (gemeinsamer Speicher, keine zusätzliche Kopie im Portal). Dmabuf
(GPU-Puffer ohne Umweg über die CPU) gibt es für diesen Weg noch nicht; bei 1440p144 ist der
Auslese-Weg der wichtigste Punkt zum Messen. Das Auslesen blockiert den Compositor nicht: Er stößt die Kopie auf der GPU an, verschickt
die Frame-Callbacks (ein Spiel auf demselben Monitor rendert also gleich weiter) und übergibt das
Bild erst, wenn eine GPU-Fence meldet, dass die Kopie fertig ist.

## Alternative: `xdg-desktop-portal-wlr`

Statt `mywm-portal` geht auch `xdg-desktop-portal-wlr` über `wlr-screencopy`; es kann nur Monitore.
Dafür in `mywm-portals.conf` `org.freedesktop.impl.portal.ScreenCast=wlr` eintragen (unter NixOS in
`xdg.portal.config.mywm`). Nötige Pakete: `xdg-desktop-portal`, `xdg-desktop-portal-wlr`, `pipewire`,
`wireplumber`.

## Grenzen

- **Fensteraufnahme** gibt es mit `mywm-portal` (siehe oben) und über `ext-foreign-toplevel-list` + `ext-image-copy-capture`
  (Toplevel-Quellen, SHM und Dmabuf). `xdg-desktop-portal-wlr` spricht nur `wlr-screencopy`
  und damit nur Monitore; für Fenster braucht es ein Portal oder Programm mit Unterstützung
  dieser Protokolle. Bis dahin: Monitor aufnehmen und in OBS zuschneiden. Der Cursor wird nur
  mit `paint_cursors` ins Bild gemalt, eigene Cursor-Sessions liefern nichts.
- Skalierte oder gedrehte Monitore werden so aufgenommen, wie sie dargestellt werden (in
  physischen Pixeln, nach der Drehung).
- Mit `copy_with_damage` (OBS, wf-recorder) wird nur bei tatsächlichen Änderungen ein neues Bild
  geliefert; bei ruhigem Bild sinkt die Rate entsprechend.

## Schnelltest ohne OBS: Vesktop/Discord

1. Prüfen, dass die Portale laufen: `systemctl --user status xdg-desktop-portal mywm-portal pipewire`.
2. In Vesktop: Sprachkanal beitreten → „Bildschirm teilen“. Das geht über Portal und PipeWire; der
   Compositor zeigt seine Auswahl (Fenster anklicken, Desktop oder Enter für den Monitor, Esc bricht ab).
3. Erst ohne Portal prüfen, ob der Compositor liefert: `grim /tmp/test.png` muss ein Bild erzeugen.
   Geht das, liegt ein Fehler bei Portal/PipeWire, nicht beim Compositor.
4. Ton beim Teilen übernimmt Vesktop selbst (Venmic), nicht das Portal.

Ohne Discord geht es auch im Browser mit der Seite „getDisplayMedia“ von webrtc.github.io/samples.
