# Streaming und Bildschirmaufnahme

Der Compositor bietet `wlr-screencopy-unstable-v1` an, das Protokoll, das
`xdg-desktop-portal-wlr` benutzt. Damit funktionieren:

- **OBS** (Quelle „Bildschirmaufnahme (PipeWire)“), **Discord**, **Browser** (Bildschirm teilen),
  über Portal und PipeWire;
- **grim**, `wf-recorder` und ähnliche Werkzeuge direkt.

GPU-Puffer (dmabuf) werden direkt beschrieben, ohne Umweg über die CPU; Shared-Memory-Puffer
brauchen einmal Auslesen der GPU. Auf der Hardware verwendet der Portal-Dienst automatisch den
dmabuf-Weg, wenn OBS/der Browser ihn versteht.

## Dynamischer Stream

Wählt man beim Teilen in der Übersicht `D`, entsteht ein Stream, dessen Inhalt man per Tastenkürzel
wechselt, ohne dass OBS oder der Browser neu teilen müssen: `cast_window` zeigt das fokussierte
Fenster, `cast_monitor` den fokussierten Monitor, `cast_clear` nichts (schwarze Bilder, nicht das
letzte Bild). Er beginnt schwarz. Die Tasten haben keine Standardbelegung (`[bindings]`, siehe
`example.toml`). Ändert sich die Größe, handelt der Stream sie mit dem Empfänger neu aus.

Die Bar zeigt währenddessen einen roten Punkt mit „Rec“, solange etwas aufgenommen wird, und beim Auswählen einen
Hinweis mit den Tasten; ein einzeln geteiltes Fenster hat einen Rahmen in `appearance.capture_border`.

## Fenster aus Aufnahmen heraushalten

Eine Fensterregel mit `block_from_capture = true` zeigt das Fenster in jeder Aufnahme als schwarze
Fläche: in Streams und Aufnahmen über das Portal oder `wlr-screencopy` (OBS, Discord, Browser,
`grim`), beim Teilen genau dieses Fensters, in der Workspace-Übersicht und in den eingebauten
Screenshots. Auf dem Bildschirm bleibt es sichtbar. Popups und Menüs des Fensters sind mit
abgedeckt, Fenster davor bleiben sichtbar. Beispiel (Passwortmanager im Browser):

```toml
[[rules]]
title = "*Bitwarden*"
block_from_capture = true
```

## Eigenes Portal: `splice-portal` (empfohlen)

`splice-portal` ersetzt `xdg-desktop-portal-wlr`. Es ist das ScreenCast-Backend von
`xdg-desktop-portal` und fragt **im Compositor** nach der Quelle: Beim Teilen (Vesktop, Discord,
Browser, OBS) öffnet sich auf allen Monitoren die Workspace-Übersicht (Rahmen in der Aufnahme-Farbe).
Ein Klick auf ein **Fenster** teilt dieses Fenster, auch eines auf einem gerade nicht gezeigten
Workspace; ein Klick in eine Kachel daneben (oder Enter) teilt den **Monitor** dieser Übersicht, `D`
einen dynamischen Stream, Esc oder ein Klick neben die Kacheln bricht ab. Die Kacheln `+` und
Scratchpad gibt es dabei nicht. Anders als beim wlr-Portal funktionieren damit auch einzelne Fenster, und es läuft
ohne `slurp`/Chooser-Programm. Die Fenstergröße wird live nachgeführt, der Cursor kann mitgemalt werden.

Außerdem ist es das Inhibit-Backend: Firefox (und Flatpak-Apps) verhindern Sperre und Monitor-Aus beim
Video über das Portal statt über Wayland. Das GTK-Portal nimmt solche Anfragen zwar an, braucht dahinter
aber eine GNOME-Sitzung; `splice-portal` gibt sie an den Compositor weiter. Dafür steht in
`splice-portals.conf` neben ScreenCast auch `org.freedesktop.impl.portal.Inhibit=splice`.

Einrichtung:

1. `cargo build --release -p splice-portal` und die Binary `splice-portal` in den `PATH` legen.
2. Aus `portal/` installieren: `splice.portal` nach `…/share/xdg-desktop-portal/portals/`,
   `org.freedesktop.impl.portal.desktop.splice.service` nach `…/share/dbus-1/services/`,
   `splice-portal.service` nach `~/.config/systemd/user/` (die D-Bus-Aktivierung startet das Portal als
   diese Unit, die mit der Sitzung endet; ohne sie bliebe ein Portal mit dem Socket einer alten
   Sitzung hängen), und die
   Portal-Auswahl `splice-portals.conf` nach `~/.config/xdg-desktop-portal/` (Sitzung und Compositor
   setzen `XDG_CURRENT_DESKTOP=splice`).
   Unter NixOS gehört `splice.portal` in ein Paket, das unter `xdg.portal.extraPortals` steht.
3. Das Portal braucht in seiner Umgebung `WAYLAND_DISPLAY` und `SPLICE_SOCKET` (derselbe Socket wie
   für die Bar, sonst `$XDG_RUNTIME_DIR/splice.sock`). Die Sitzung sollte sie dem D-Bus bekanntmachen:
   `dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP SPLICE_SOCKET`.
4. `xdg-desktop-portal-wlr` nicht mehr starten. Nötig bleiben `xdg-desktop-portal`, `pipewire`, `wireplumber`.

Ruhige Bilder: Steht das Bild still, liefert der Compositor trotzdem etwa alle 0,4 s ein Bild (Keepalive),
sonst hält Chromium (Discord, Vesktop) den Strom für tot. Öffnet eine App zwei Sitzungen gleichzeitig (Vesktop tut das),
beantwortet die Auswahl beide; die zweite wird nicht abgelehnt.

Logs: `journalctl --user -u splice-portal` bzw. die Ausgabe von `splice-portal` zeigt Format, Bildgröße,
Verbraucher und Bilder pro 5 s; im Compositor-Log stehen `capture session: …` (Quelle, Größe, erstes Bild, Ende).

Technik: Das Portal hängt als Wayland-Client am Compositor (`ext-image-copy-capture`) und schreibt
die Bilder direkt in PipeWire-Puffer (gemeinsamer Speicher, keine zusätzliche Kopie im Portal). Dmabuf
(GPU-Puffer ohne Umweg über die CPU) gibt es für diesen Weg noch nicht; bei 1440p144 ist der
Auslese-Weg der wichtigste Punkt zum Messen. Das Auslesen blockiert den Compositor nicht: Er stößt die Kopie auf der GPU an, verschickt
die Frame-Callbacks (ein Spiel auf demselben Monitor rendert also gleich weiter) und übergibt das
Bild erst, wenn eine GPU-Fence meldet, dass die Kopie fertig ist.

## Alternative: `xdg-desktop-portal-wlr`

Statt `splice-portal` geht auch `xdg-desktop-portal-wlr` über `wlr-screencopy`; es kann nur Monitore.
Dafür in `splice-portals.conf` `org.freedesktop.impl.portal.ScreenCast=wlr` eintragen (unter NixOS in
`xdg.portal.config.splice`). Nötige Pakete: `xdg-desktop-portal`, `xdg-desktop-portal-wlr`, `pipewire`,
`wireplumber`.

## Grenzen

- **Fensteraufnahme** gibt es mit `splice-portal` (siehe oben) und über `ext-foreign-toplevel-list` + `ext-image-copy-capture`
  (Toplevel-Quellen, SHM und Dmabuf). `xdg-desktop-portal-wlr` spricht nur `wlr-screencopy`
  und damit nur Monitore; für Fenster braucht es ein Portal oder Programm mit Unterstützung
  dieser Protokolle. Bis dahin: Monitor aufnehmen und in OBS zuschneiden. Der Cursor wird nur
  mit `paint_cursors` ins Bild gemalt, eigene Cursor-Sessions liefern nichts.
- Skalierte oder gedrehte Monitore werden so aufgenommen, wie sie dargestellt werden (in
  physischen Pixeln, nach der Drehung).
- Mit `copy_with_damage` (OBS, wf-recorder) wird nur bei tatsächlichen Änderungen ein neues Bild
  geliefert; bei ruhigem Bild sinkt die Rate entsprechend.

## Schnelltest ohne OBS: Vesktop/Discord

1. Prüfen, dass die Portale laufen: `systemctl --user status xdg-desktop-portal splice-portal pipewire`.
2. In Vesktop: Sprachkanal beitreten → „Bildschirm teilen“. Das geht über Portal und PipeWire; der
   Compositor zeigt seine Auswahl (Fenster anklicken, Desktop oder Enter für den Monitor, Esc bricht ab).
3. Erst ohne Portal prüfen, ob der Compositor liefert: `grim /tmp/test.png` muss ein Bild erzeugen.
   Geht das, liegt ein Fehler bei Portal/PipeWire, nicht beim Compositor.
4. Ton beim Teilen übernimmt Vesktop selbst (Venmic), nicht das Portal.

Ohne Discord geht es auch im Browser mit der Seite „getDisplayMedia“ von webrtc.github.io/samples.
