# Einstellungen per GUI: `mywm-settings`

Ein eigenständiges Fenster (egui) zum Ändern von Einstellungen und Tastenbelegung. Es ist ein
eigenes Binary und im Flake ein eigenes Paket (`packages.mywm-settings`), liegt aber in diesem
Repo, weil es Schema, Prüfung und Tastennamen des Compositors (`mywm-config`) mitbenutzt.

```sh
cargo run -p mywm-settings -- [--config /pfad/zur/config.toml]
```

## Wie es mit der Config umgeht
- Es bearbeitet **die Config-Datei selbst** mit `toml_edit`: Kommentare, Reihenfolge und alles, was die GUI
  nicht kennt (z. B. `[[outputs]]`), bleiben unverändert.
- **Dotfiles und Nix:** Liegt `~/.config/mywm/config.toml` als Symlink auf eine Datei in deinem
  Dotfiles-Ordner (`mkOutOfStoreSymlink`), schreibt die GUI in diese Datei. Der Symlink bleibt, Git sieht die Änderung,
  und ein `switch` überschreibt nichts. Ein Pfad im Nix-Store (schreibgeschützt) wird erkannt; dann den Dotfiles-Pfad
  unter „Datei“ eintragen (er wird gemerkt, in `~/.config/mywm-settings/config-path`).
- **Prüfung:** Vor dem Speichern prüft dieselbe Funktion wie im Compositor die Datei. Ungültiges
  (Werte außerhalb der Grenzen, doppelte Tasten, unbekannte Tasten) lässt sich nicht speichern; der Grund steht oben.
- **Live:** Nach dem Speichern bekommt der laufende Compositor den Befehl zum Neuladen (über `MYWM_SOCKET`,
  sonst `$XDG_RUNTIME_DIR/mywm.sock`). Felder mit „(Neustart)“ (Monitore, Idle, VRR, Xwayland) wirken erst
  nach dem nächsten Start; die GUI sagt es nach dem Speichern.

## Seiten
Tastenbelegung (mit „+ Taste“: Modifikatoren ankreuzen und die Taste drücken; Super kann ein Fenster nicht
erkennen, darum die Kästchen), Programme (Tasten, die Programme starten), Fenster, Fensterregeln, Effekte, Eingabe,
Aussehen, Allgemein, Datei. Geänderte Werte sind fett, „Standard“ entfernt sie wieder aus der Datei.

Monitore stellt man weiter mit kanshi oder `wlr-randr` ein.
