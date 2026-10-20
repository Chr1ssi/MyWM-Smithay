# Eingebaute Shell

Mit `builtin_shell = true` (oberste Ebene der Config, gilt nach Neustart) zeichnet der Compositor die Bar,
das Powermenü und die Hinweise oben rechts selbst, statt die Quickshell-Bar von mywm-shell zu starten
(`--bar` startet dann nichts). Aussehen, Maße, Farben und Texte entsprechen der Quickshell-Bar
(`bar.qml`, `BarButton.qml`, `BarIsland.qml`, `Theme.qml`, `I18n.qml`). Standard ist weiterhin die
Quickshell-Bar, bis die eingebaute Shell alles kann.

## Was sie schon kann

- **Bar** auf jedem Monitor: Workspaces (Klick wechselt, `+` legt einen an, `S` für das Scratchpad),
  Medien-Pill (Titel des ersten MPRIS-Players, der spielt), Datum und Uhrzeit (`LC_TIME`, CLDR-Abkürzungen wie
  Qt), Aufnahme-Anzeige, Commandcenter-Symbol (Bluetooth an oder verbunden, Nachtlicht, sonst Zahnrad),
  Lautstärke des Standardausgangs (Mausrad: 5 % je Raste, Rechtsklick: stumm) und Power-Knopf. Sie hält die oberen
  40 Pixel frei wie früher ihre Exclusive Zone; Vollbildfenster und `Top`-Layer anderer Programme liegen davor.
- **Powermenü**: Sperren, Abmelden, Neustart, Ausschalten; Fünf-Sekunden-Countdown mit Ring, Escape, Klick
  daneben oder „Abbrechen“ stoppen ihn, Tab/Enter/Leertaste bedienen es. Das Exit-Binding öffnet es mit dem
  Countdown zum Abmelden. Solange es offen ist, gehören ihm die Tasten (außer den Bindings).
- **Hinweise** oben rechts auf dem ersten Monitor: Auswahl beim Teilen, Farbpipette mit Farbfeld,
  fehlerhafte Konfiguration (mit Schließen-Knopf).
- Theme aus `theme.json` (sonst `[appearance]`), neu geladen mit dem Config-Reload/Theme-Reload des Pickers.
  Sprache nach `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG` (Deutsch, sonst Englisch).

- **Nachtlicht**: steht in `$XDG_STATE_HOME/mywm/night-light` „1“, läuft `gammastep -O 4000` wie bei der
  Quickshell-Bar (nur auf echter Hardware); umschalten lässt es sich erst mit dem Commandcenter.

## Was noch fehlt

Tray, Benachrichtigungsdienst und -verlauf, die Panels (Audio, Medien, Commandcenter, Kalender), Launcher,
Wallpaper-Picker, Sperr- und Anmeldebildschirm. Linksklicks auf Uhr, Medien und Status-Knöpfe tun noch
nichts, die Glocke zeigt keine Zahl. Datum und Uhrzeit kennen nur deutsche und englische Namen.

## Aufbau

- `crates/mywm-ui` (ohne Compositor-Abhängigkeit, mit Tests): Szenen aus Rechtecken, Texten und Bögen
  (`bar`, `power`, `notices`), Zeichnen auf der CPU (tiny-skia, Text mit cosmic-text und den Schriften aus
  fontconfig), Theme, Texte, Uhr.
- `src/shell.rs`: füllt die Szenen aus dem Zustand des Compositors, zeichnet sie nur bei Änderungen neu
  (dieselben Änderungen, die das Bar-Protokoll meldet, dazu Hover, Uhr, Countdown) und legt die Bilder als
  Texturen in den Frame. Die Schriften laden beim Start in einem eigenen Thread.
- Reihenfolge von vorn nach hinten: Powermenü, `Overlay`-Layer, Hinweise, Vollbildfenster, `Top`-Layer,
  Bar, Fenster. Zeiger und Klicks folgen derselben Reihenfolge (`pointer_target_over` in `src/input.rs`).

- `mywm-shell-services` (`crates/mywm-shell-services`, eigenes Programm neben dem Compositor): fragt die
  Dienste ab, damit ein hängender oder abstürzender Dienst nie Bild oder Eingabe einfriert. PipeWire
  (Standardausgang aus der `default`-Metadaten, Lautstärke kubisch wie bei `wpctl` und Quickshell), MPRIS auf dem
  Session-Bus und BlueZ auf dem System-Bus laufen in je einem Thread und verbinden sich bei Ausfall nach drei
  Sekunden neu. Lautstärke und Stummschaltung setzt es mit `wpctl` (WirePlumber), wie das Audio-Panel der
  Quickshell-Shell; schnelles Scrollen fasst es zu einem Aufruf zusammen.
- `crates/mywm-shell-protocol`: eine JSON-Zeile je Nachricht über stdin/stdout. Der Dienstprozess schickt bei
  jeder Änderung den ganzen Status, der Compositor Befehle. Der Compositor liest ohne zu blockieren, verwirft
  Befehle, die der Prozess nicht abnimmt, und startet ihn nach dem Ende mit wachsender Pause neu (1 bis 30
  Sekunden; nach einer Minute Laufzeit wieder kurz). Stirbt der Compositor, endet der Prozess mit ihm
  (`PR_SET_PDEATHSIG`). Statusänderungen protokolliert er mit `RUST_LOG=mywm_compositor::shell_services=debug`.

Nächste Schritte im selben Prozess: Benachrichtigungsdienst, Tray (StatusNotifierItem), dann PAM und greetd.

## Test

`tests/builtin_shell_smoke.py`: Bar gezeichnet und Fenster darunter, Workspace-Knöpfe, Powermenü,
abgebrochener und abgelaufener Logout-Countdown, Konfigurationshinweis.
`tests/shell_services_smoke.py`: mit eigenem PipeWire (Null-Sink), nachgebautem Player (`tests/fake_mpris.py`) und
nachgebautem BlueZ (`tests/fake_bluez.py`): Mausrad und Rechtsklick auf der Lautstärke, Medientitel, Bluetooth,
Neustart des Dienstprozesses nach `kill -9`. Nested meldet winit Drücken und Loslassen einer X-Mausradtaste je als
eine Raste; ein Klick dort sind also zwei.
