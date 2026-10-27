# Einstellungen per GUI: `splice-settings`

Ein eigenständiges Fenster (egui) zum Ändern von Einstellungen und Tastenbelegung. Es ist ein
eigenes Binary und im Flake ein eigenes Paket (`packages.splice-settings`), liegt aber in diesem
Repo, weil es Schema, Prüfung und Tastennamen des Compositors (`splice-config`) mitbenutzt.

```sh
cargo run -p splice-settings -- [--config /pfad/zur/config.toml]
```

## Wie es mit der Config umgeht
- Es bearbeitet **die Config-Datei selbst** mit `toml_edit`: Kommentare, Reihenfolge und alles, was die GUI
  nicht kennt (z. B. `[[outputs]]`), bleiben unverändert.
- **Dotfiles und Nix:** Liegt `~/.config/splice/config.toml` als Symlink auf eine Datei in deinem
  Dotfiles-Ordner (`mkOutOfStoreSymlink`), schreibt die GUI in diese Datei. Der Symlink bleibt, Git sieht die Änderung,
  und ein `switch` überschreibt nichts. Ein Pfad im Nix-Store (schreibgeschützt) wird erkannt; dann den Dotfiles-Pfad
  unter „Datei“ eintragen (er wird gemerkt, in `~/.config/splice-settings/config-path`).
- **Prüfung:** Vor dem Speichern prüft dieselbe Funktion wie im Compositor die Datei. Ungültiges
  (Werte außerhalb der Grenzen, doppelte Tasten, unbekannte Tasten) lässt sich nicht speichern; der Grund steht oben.
- **Live:** Nach dem Speichern bekommt der laufende Compositor den Befehl zum Neuladen (über `SPLICE_SOCKET`,
  sonst `$XDG_RUNTIME_DIR/splice.sock`). Felder mit „(Neustart)“ (Monitore, Idle, VRR, Xwayland, eingebaute Shell) wirken erst
  nach dem nächsten Start; die GUI sagt es nach dem Speichern.

## Seiten
Tastenbelegung (mit „+ Taste“: Modifikatoren ankreuzen und die Taste drücken; Super kann ein Fenster nicht
erkennen, darum die Kästchen), Programme (Tasten, die Programme starten), Fenster, Fensterregeln, Effekte, Eingabe,
Aussehen, Allgemein, Datei. Geänderte Werte sind in der Akzentfarbe geschrieben, „Standard“ entfernt sie wieder aus
der Datei. Die Einstellungen stehen auf abgerundeten Karten in einer Spalte in der Mitte; Farben (aus `[appearance]`,
vom Wallpaper-Theme in `theme.json` überschrieben, wie in der Shell) und Systemschrift (`fc-match "Sans Serif"`)
sind die der Shell. Fehler und der Stand des Speicherns stehen unten im Fenster, damit sich der Inhalt nicht
verschiebt.

Aus dem Commandcenter (Zeile „Einstellungen“) startet die Shell `splice-settings` aus dem `PATH`.

Monitore stellt man weiter mit kanshi oder `wlr-randr` ein.
