# Eingebaute Shell

Mit `builtin_shell = true` (oberste Ebene der Config, gilt nach Neustart) zeichnet der Compositor die Bar,
das Powermenü und die Hinweise oben rechts selbst, statt die Quickshell-Bar von mywm-shell zu starten
(`--bar` startet dann nichts). Aussehen, Maße, Farben und Texte entsprechen der Quickshell-Bar
(`bar.qml`, `BarButton.qml`, `BarIsland.qml`, `Theme.qml`, `I18n.qml`). Standard ist weiterhin die
Quickshell-Bar, bis die eingebaute Shell alles kann.

## Was sie schon kann

- **Bar** auf jedem Monitor: Workspaces (Klick wechselt, `+` legt einen an, `S` für das Scratchpad),
  Datum und Uhrzeit (`LC_TIME`, CLDR-Abkürzungen wie Qt), Aufnahme-Anzeige, Power-Knopf. Sie hält die oberen
  40 Pixel frei wie früher ihre Exclusive Zone; Vollbildfenster und `Top`-Layer anderer Programme liegen davor.
- **Powermenü**: Sperren, Abmelden, Neustart, Ausschalten; Fünf-Sekunden-Countdown mit Ring, Escape, Klick
  daneben oder „Abbrechen“ stoppen ihn, Tab/Enter/Leertaste bedienen es. Das Exit-Binding öffnet es mit dem
  Countdown zum Abmelden. Solange es offen ist, gehören ihm die Tasten (außer den Bindings).
- **Hinweise** oben rechts auf dem ersten Monitor: Auswahl beim Teilen, Farbpipette mit Farbfeld,
  fehlerhafte Konfiguration (mit Schließen-Knopf).
- Theme aus `theme.json` (sonst `[appearance]`), neu geladen mit dem Config-Reload/Theme-Reload des Pickers.
  Sprache nach `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG` (Deutsch, sonst Englisch).

## Was noch fehlt

Medien-Pill, Tray, Benachrichtigungsdienst und -verlauf, Audio, Commandcenter (Bluetooth, Nachtlicht,
Adressen), Kalender, Launcher, Wallpaper-Picker, Sperr- und Anmeldebildschirm. Die Knöpfe für Commandcenter,
Benachrichtigungen und Audio zeigen bis dahin, was die Quickshell-Bar ohne diese Dienste zeigt; Klicks auf
sie und auf die Uhr tun noch nichts. Datum und Uhrzeit kennen nur deutsche und englische Namen.

## Aufbau

- `crates/mywm-ui` (ohne Compositor-Abhängigkeit, mit Tests): Szenen aus Rechtecken, Texten und Bögen
  (`bar`, `power`, `notices`), Zeichnen auf der CPU (tiny-skia, Text mit cosmic-text und den Schriften aus
  fontconfig), Theme, Texte, Uhr.
- `src/shell.rs`: füllt die Szenen aus dem Zustand des Compositors, zeichnet sie nur bei Änderungen neu
  (dieselben Änderungen, die das Bar-Protokoll meldet, dazu Hover, Uhr, Countdown) und legt die Bilder als
  Texturen in den Frame. Die Schriften laden beim Start in einem eigenen Thread.
- Reihenfolge von vorn nach hinten: Powermenü, `Overlay`-Layer, Hinweise, Vollbildfenster, `Top`-Layer,
  Bar, Fenster. Zeiger und Klicks folgen derselben Reihenfolge (`pointer_target_over` in `src/input.rs`).

Nächster Schritt: ein Dienstprozess (`mywm-compositor --shell-services`) für D-Bus (Benachrichtigungen,
Tray, MPRIS, BlueZ), PipeWire, PAM und greetd. Er tauscht Zustand und Befehle mit dem Compositor aus und wird
nach einem Absturz neu gestartet, damit ein hängender Dienst nie Bild oder Eingabe einfriert.

## Test

`tests/builtin_shell_smoke.py`: Bar gezeichnet und Fenster darunter, Workspace-Knöpfe, Powermenü,
abgebrochener und abgelaufener Logout-Countdown, Konfigurationshinweis.
