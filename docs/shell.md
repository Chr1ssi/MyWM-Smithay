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

- **Benachrichtigungen**: Die Shell ist selbst der Freedesktop-Benachrichtigungsdienst (Dunst wird nicht
  gebraucht; läuft ein anderer Dienst, wartet sie, bis er den Namen freigibt). Toasts erscheinen rechts oben auf dem
  ersten Monitor, höchstens vier, mit Bild oder App-Symbol, Fett/Kursiv/Unterstrichen aus der Markup, Aktionen und
  „×“. Ein Toast verschwindet nach der Zeit, die die App angibt, sonst nach 4 Sekunden (niedrige Dringlichkeit) oder
  7 Sekunden, mit 0 bleibt er stehen; unter dem Mauszeiger bleibt er, danach läuft seine Zeit neu. Während ein Spiel
  im Vollbild läuft, erscheinen nur kritische Meldungen, die anderen warten in der Liste. Die Glocke zeigt die Zahl
  und öffnet die Benachrichtigungszentrale (rechts oben, Liste zum Scrollen, „Alle löschen“); ein Klick hinein und
  dann Escape oder ein Klick daneben schließen sie, solange sie offen ist, ruhen die Toasts. Eine Aktion schließt die
  Meldung, außer sie ist `resident`. Startet der Dienstprozess neu, ist die Liste leer.
- **Tray** (StatusNotifierItem): Die Shell ist selbst der Watcher (läuft schon einer, zeigt sie dessen Einträge und
  übernimmt, sobald er endet). Passive Einträge sind ausgeblendet, Aufmerksamkeit bekommt einen Ring. Linksklick
  aktiviert (oder öffnet das Menü, wenn der Eintrag nur ein Menü hat), Mittelklick ruft die zweite Aktivierung,
  Rechtsklick öffnet das Menü, das Mausrad scrollt. Das Menü (DBusMenu) erscheint rechts unter der Bar mit Haken und
  Radioknöpfen, Symbolen (symbolische als Glyphen), unterstrichenem Kürzelbuchstaben und Untermenüs mit „←“ zurück;
  Escape nach einem Klick hinein oder ein Klick daneben schließt es. Overlay-Symbole zeichnet die Shell nicht ein,
  und ein Menü, das sich ändert, während es offen ist, zeigt die Änderung erst beim nächsten Öffnen.
- **Panels** unter der Bar wie bei Quickshell (rechts für Audio, Commandcenter und Benachrichtigungen, mittig für
  Medien und Kalender), eins zur Zeit; ein Klick hinein gibt ihm die Tastatur (Escape schließt), ein Klick daneben
  schließt es, das Mausrad scrollt es, wenn der Monitor zu niedrig ist.
- **Commandcenter**: Kacheln für Bluetooth und Nachtlicht, die lokalen IPv4-Adressen (alle fünf Sekunden über `ip`,
  solange das Panel offen ist; ein Klick kopiert eine mit `wl-copy`) und die Bluetooth-Geräte mit Akkustand zum
  Verbinden und Trennen; „Suchen“ findet neue, „Koppeln“ vertraut ihnen zuerst, damit sie sich später selbst verbinden.
  Das Nachtlicht hält `gammastep -O 4000` am Laufen, solange es an ist, und merkt sich die Wahl in
  `$XDG_STATE_HOME/mywm/night-light`.
- **Audio-Panel**: Standardausgang und -mikrofon mit Regler (ziehen) und Prozentknopf (stumm), die übrigen hinter
  „Anderen Ausgang wählen“/„Anderes Mikrofon wählen“ (ein Klick macht sie zum Standard), dazu die Anwendungen, die
  spielen. WirePlumbers Smart-Filter (der NixOS-Mikrofonfilter) fehlen in der Liste, wie bei Quickshell.
- **Medien-Panel**: alle MPRIS-Player mit Cover, Titel, Interpret und Zurück/Abspielen/Weiter; was ein Player nicht
  erlaubt, ist ausgegraut. Cover kommen aus `file://`-Adressen; Cover aus dem Netz (`https://`) lädt die Shell nicht.
- **Kalender-Panel** (Klick auf die Uhr): Monat mit „‹“/„›“, Tage mit bis zu drei Punkten in den Farben ihrer
  Termine, die Termine des gewählten Tages und die nächsten acht, „Kalender-App öffnen“ (`foot … mywm-calendar app`).
  Die Termine liefert `mywm-calendar events <Monatserster> <8. des Folgemonats>` (khal), die Kalender
  `mywm-calendar calendars`, jeweils in einem eigenen Thread; jedes Öffnen beginnt wie bei Quickshell neu beim
  heutigen Tag. „+“ öffnet das Formular (Titel, Start, Ende, Ort, Kalender) für `mywm-calendar create`; Felder und
  Kalenderauswahl sehen aus wie Qt Quicks Fusion-Steuerelemente, mit der Schrift, die fontconfig für „Sans Serif“
  wählt (`fc-match`, wie Qt).
- **Nachtlicht**: steht in `$XDG_STATE_HOME/mywm/night-light` „1“, läuft `gammastep -O 4000` wie bei der
  Quickshell-Bar (nur auf echter Hardware); umschalten lässt es sich erst mit dem Commandcenter.

## Was noch fehlt

Launcher, Wallpaper-Picker, Sperr- und Anmeldebildschirm. In den Panels führt Tab noch nicht von Knopf zu Knopf
(bei Quickshell erreichte Tab jeden Knopf); im Kalenderformular wechselt Tab nur zwischen den vier Feldern. Die
Felder kennen Tippen, Löschen, Pfeile, Pos1/Ende und Klicks, aber keine Auswahl, keine Zwischenablage und keine
Tastenwiederholung, und der Cursor blinkt nicht. Datum und Uhrzeit
kennen nur deutsche und englische Namen. Links in Benachrichtigungen erscheinen unterstrichen in Textfarbe, nicht
wie bei Qt blau.

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
  (Ausgänge, Mikrofone und Anwendungsströme mit Lautstärke, kubisch wie bei `wpctl` und Quickshell, und die beiden
  Standards aus der `default`-Metadaten), MPRIS auf dem
  Session-Bus und BlueZ auf dem System-Bus laufen in je einem Thread und verbinden sich bei Ausfall nach drei
  Sekunden neu. Lautstärke und Stummschaltung setzt es mit `wpctl` (WirePlumber), wie das Audio-Panel der
  Quickshell-Shell; schnelles Scrollen fasst es zu einem Aufruf zusammen.
- Benachrichtigungen im Dienstprozess: `org.freedesktop.Notifications` (Fähigkeiten wie bei Quickshell:
  `actions`, `body`, `body-markup`, `icon-static`, `persistence`). Bilder kommen aus `image-data`, `image-path`,
  dem App-Symbol oder dem Symbol des `desktop-entry`; Symbole sucht er im Icon-Theme aus GTKs `settings.ini`,
  dessen Vorfahren, `hicolor` und `pixmaps` (SVG über resvg). Er dekodiert sie in einem eigenen Thread und
  verkleinert sie auf höchstens 128 Pixel.
- `crates/mywm-shell-protocol`: eine JSON-Zeile je Nachricht über stdin/stdout. Der Dienstprozess schickt bei
  jeder Änderung den ganzen Status und jede Benachrichtigung einzeln (Bilder als Base64), der Compositor Befehle. Der Compositor liest ohne zu blockieren, verwirft
  Befehle, die der Prozess nicht abnimmt, und startet ihn nach dem Ende mit wachsender Pause neu (1 bis 30
  Sekunden; nach einer Minute Laufzeit wieder kurz). Stirbt der Compositor, endet der Prozess mit ihm
  (`PR_SET_PDEATHSIG`). Statusänderungen protokolliert er mit `RUST_LOG=mywm_compositor::shell_services=debug`.
  Eine Lautstärke, die die Bar verlangt hat, zeigt sie, bis ein Bericht sie bestätigt (höchstens eine Sekunde):
  Berichte von vor dem Befehl würden sonst zurückspringen lassen, und die nächste Raste zählte vom alten Wert.
- `crates/mywm-ui/src/notifications.rs`: die Karte (`NotificationCard.qml`) und die Zentrale
  (`NotificationCenter.qml` im Panelrahmen von `bar.qml`); `src/shell_notifications.rs` hält Liste, Toasts und
  Zentrale. cosmic-text 0.19 lässt beim Kürzen der letzten erlaubten Zeile das Leerzeichen des Umbruchs vor ihr
  stehen; `text.rs` beginnt solche Folgezeilen beim ersten Wort (Workaround, bis cosmic-text das selbst richtig
  macht).

- `crates/mywm-ui/src/calendar.rs`: Inhalt des Kalender-Panels (`CalendarPanelInteractive.qml`) mit den
  Fusion-Feldern; `src/shell_calendar.rs` ruft `mywm-calendar` auf, hält Monat, Termine und Formular und nimmt die
  Tasten der Felder.

Nächste Schritte: im Dienstprozess PAM und greetd (Sperr- und Anmeldebildschirm), Launcher, Wallpaper-Picker.

## Test

`tests/builtin_shell_smoke.py`: Bar gezeichnet und Fenster darunter, Workspace-Knöpfe, Powermenü,
abgebrochener und abgelaufener Logout-Countdown, Konfigurationshinweis.
`tests/shell_services_smoke.py`: mit eigenem PipeWire (Null-Sink), nachgebautem Player (`tests/fake_mpris.py`) und
nachgebautem BlueZ (`tests/fake_bluez.py`): Mausrad und Rechtsklick auf der Lautstärke, Medientitel, Bluetooth,
Neustart des Dienstprozesses nach `kill -9`.
`tests/control_smoke.py`: nachgebautes BlueZ und Platzhalter für `gammastep`, `ip`, `wl-copy`, geklickt an denselben
Stellen wie mywm-shells Test des Quickshell-Panels (das Layout ist also gleich): Bluetooth an, verbinden, suchen,
koppeln, Adresse kopieren, Nachtlicht an und aus, gemerktes Nachtlicht beim nächsten Start.
`tests/audio_smoke.py`: eigenes PipeWire mit zwei Ausgängen, Mikrofon, Smart-Filter und Anwendungsstrom: Regler
ziehen, Stummschalten, anderen Ausgang wählen. Die PipeWire-Tests starten WirePlumber ohne Hardware-Monitore
(`hermetic_wireplumber` in `tests/smoke_support.py`): sonst fände es die echten Geräte des Rechners, und ein Test,
der die Lautstärke des Standardausgangs setzt, könnte die eines echten Geräts setzen.
`tests/tray_smoke.py`: nachgebauter Tray-Eintrag (`tests/fake_tray.py`) mit Menü: Symbol in der Bar, Aktivieren,
zweite Aktivierung, Scrollen, Menü mit Untermenü und verstecktem Eintrag, Klick auf einen Eintrag, Eintrag endet.
`tests/media_smoke.py`: nachgebauter Player mit Cover, Medien-Panel mittig, Abspielen/Pause/Weiter und ein
nicht erlaubtes Zurück.
`tests/calendar_smoke.py`: Platzhalter für `mywm-calendar` und `foot`: die Uhr öffnet das Panel (Termine des Monats
und Kalender abgefragt), ein Termin ohne Titel wird nicht angelegt, getippter Titel und gewählter Kalender gehen an
`mywm-calendar create`, danach lädt es neu; Monat vor und zurück, Kalender-App öffnen schließt das Panel.
`tests/notifications_smoke.py`: auf eigenem Session-Bus Toast mit Ablaufzeit und Festhalten, „×“, Aktion und
`CloseNotification` mit den passenden D-Bus-Signalen, Zentrale über die Glocke mit „Alle löschen“, Escape und
Klick daneben. Nested meldet winit Drücken und Loslassen einer X-Mausradtaste je als
eine Raste; ein Klick dort sind also zwei.
