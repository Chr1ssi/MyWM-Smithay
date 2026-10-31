# Eingebaute Shell

Der Compositor zeichnet die Shell selbst: Bar, Powermenü, Hinweise oben rechts, Launcher, Wallpaper-Picker,
Sperrbildschirm, Anmeldebildschirm, Benachrichtigungen und Tray. Sie ist an, solange `builtin_shell = true` in der
obersten Ebene der Config steht (Standard; gilt nach Neustart). Mit `builtin_shell = false` zeichnet der Compositor
keine Shell, also auch keine Bar und keinen Sperrbildschirm; das ist nur für die nested Tests gedacht. Maße,
Farben und Texte stammen von der früheren Quickshell-Shell (`bar.qml`, `BarButton.qml`, `BarIsland.qml`,
`Theme.qml`, `I18n.qml`), die es nicht mehr gibt. Die Schrift ist die Systemschrift (die Familie, die fontconfig für
„Sans Serif“ wählt, `fc-match`); Nerd-Font-Symbole in Texten setzt die Shell immer aus „IosevkaTerm Nerd Font
Propo“, das dafür installiert sein muss. Die Panels gliedern ihren Inhalt in abgerundete Karten (Radien in
`crates/splice-ui/src/theme.rs`).

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
- **Stil** `[appearance] style = "cyberdeck"` (Standard `default`): Alle Flächen der Shell, die sonst gerundet sind
  (Bar-Inseln, Panels, Launcher, Karten, Knöpfe), bekommen stattdessen abgeschrägte Ecken oben links und unten rechts
  (die Abschrägung ist der halbe Radius, 3 bis 14 Pixel; Radien unter 3 bleiben eckig). Die Flächen der Shell sind
  fast undurchsichtig (90 %) und tragen eine durchgehende Linie in der Linienfarbe des Farbschemas. Dasselbe Stilmittel
  gilt für alles, was `splice-ui` zeichnet, auch Sperrbildschirm und Anmeldebildschirm. Die Farben kommen weiter aus dem
  Wallpaper: Der Generator (`splice-theme`) lässt die Akzente mit hoher Chroma (Primär, Sekundär, Tertiär bei Ton 66 bis
  74) auf Flächen leuchten, die Richtung Schwarz sinken (Oberfläche Ton 4 statt 7), zeichnet die Linien im Farbton
  des Primärakzents und färbt auch die Terminalfarben kräftiger; Terminal, GTK, Vesktop und Neovim folgen, weil sie
  aus derselben `theme.json` erzeugt werden. Alle Kontraste des normalen Farbschemas bleiben erhalten. Beim Wechsel des
  Stils (Config-Reload) erzeugt der Compositor das Farbschema aus dem gewählten Wallpaper neu und lädt danach die Shell.
  Dazu kommen Kleinigkeiten: In der Bar heißen die Workspaces `01`, `02`, …, der ausgewählte Knopf ist solide in der
  Akzentfarbe gefüllt (dunkle Schrift), die Uhr steht in Großbuchstaben, und die Lautstärke zeigt acht Segmente
  (`▮▮▮▮▮▯▯▯ 62%`). Der Launcher trägt den Titel in der Akzentfarbe (`ANWENDUNGEN //`) und umrandet die ausgewählte
  Zeile mit einer Akzentlinie. Jede Benachrichtigung hat oben eine Kennung (`MSG·07 · APP`, bei kritischen in der
  Warnfarbe), die den Namen der App ersetzt, der sonst unten steht.
  Sperr- und Anmeldebildschirm bekommen eine Kennungszeile in der Karte (`SPLICE // LOCK` oder `SPLICE // LOGIN`, daneben
  die Monitorgröße), das Datum in Großbuchstaben und ein feines Zeilenraster (Scanlines: jede dritte Zeile ein dunkler
  Strich) über dem abgedunkelten Bild, auf jedem Monitor. Der Anmeldebildschirm zeigt auf Monitoren ab 1112 Pixel Breite links
  neben der Karte den Ablauf einer Anmeldung als Pseudocode (reine Dekoration) und rechts Statuszeilen, die stimmen:
  Monitorgröße, gewählte Sitzung und ob er auf Eingaben wartet, prüft (`checking`) oder das Passwort abgelehnt hat
  (`denied`). Der Anmeldebildschirm liest die Config aus `programs.splice.greeter.settings`: Dort muss
  `appearance.style` stehen, damit er den Stil zeigt; die Farben kommen wie sonst aus `theme.json` im
  `greeterDirectory`.
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
- **Panels** unter der Bar (rechts für Audio, Commandcenter und Benachrichtigungen, mittig für
  Medien und Kalender), eins zur Zeit; ein Klick hinein gibt ihm die Tastatur (Escape schließt), ein Klick daneben
  schließt es, das Mausrad scrollt es, wenn der Monitor zu niedrig ist.
- **Commandcenter**: Kacheln für Bluetooth und Nachtlicht, die lokalen IPv4-Adressen (alle fünf Sekunden über `ip`,
  solange das Panel offen ist; ein Klick kopiert eine mit `wl-copy`) und die Bluetooth-Geräte mit Akkustand zum
  Verbinden und Trennen; „Suchen“ findet neue, „Koppeln“ vertraut ihnen zuerst, damit sie sich später selbst verbinden.
  Das Nachtlicht hält `gammastep -O 4000` am Laufen, solange es an ist, und merkt sich die Wahl in
  `$XDG_STATE_HOME/splice/night-light`. Die letzte Zeile „Einstellungen“ startet `splice-settings` (aus dem `PATH`) und
  schließt das Panel.
- **Audio-Panel**: Standardausgang und -mikrofon mit Regler (ziehen) und Prozentknopf (stumm), die übrigen hinter
  „Anderen Ausgang wählen“/„Anderes Mikrofon wählen“ (ein Klick macht sie zum Standard), dazu die Anwendungen, die
  spielen. WirePlumbers Smart-Filter (der NixOS-Mikrofonfilter) fehlen in der Liste, wie bei Quickshell.
- **Medien-Panel**: alle MPRIS-Player mit Cover, Titel, Interpret und Zurück/Abspielen/Weiter; was ein Player nicht
  erlaubt, ist ausgegraut. Cover kommen aus `file://`-Adressen; Cover aus dem Netz (`https://`) lädt die Shell nicht.
- **Kalender-Panel** (Klick auf die Uhr): Monat mit „‹“/„›“, Tage mit bis zu drei Punkten in den Farben ihrer
  Termine, die Termine des gewählten Tages und die nächsten acht, „Kalender-App öffnen“ (`foot … splice-calendar app`).
  Die Termine liefert `splice-calendar events <Monatserster> <8. des Folgemonats>` (khal), die Kalender
  `splice-calendar calendars`, jeweils in einem eigenen Thread; jedes Öffnen beginnt wie bei Quickshell neu beim
  heutigen Tag. „+“ öffnet das Formular (Titel, Start, Ende, Ort, Kalender) für `splice-calendar create`; Felder und
  Kalenderauswahl sind abgerundete Kästen im Stil der Shell (Akzentrand beim Fokus, die offene Liste liegt undurchsichtig
  darüber).
- **Launcher** (Launcher-Binding, solange `launcher` in der Konfiguration nicht auf ein anderes Programm zeigt):
  Karte in der Mitte des Monitors unter dem Zeiger mit Suchfeld; jedes getippte Wort muss in Name, generischem
  Namen, Kommentar, Id oder Schlüsselwörtern vorkommen, Namen mit der Suche am Anfang stehen vorn. Pfeile und Tab
  wählen, Enter oder ein Klick startet (mit `Path=` als Verzeichnis, mit `Terminal=true` im Terminal der
  Konfiguration plus `-e`), Escape schließt. Die Einträge liest der Dienstprozess wie Quickshells `DesktopEntries`
  (XDG-Verzeichnisse, `Name[de]` nach `LC_MESSAGES`/`LANG`, `Hidden`, `NoDisplay`, Exec-Zitate und Feldcodes) beim
  Start und bei jedem Öffnen neu, die Symbole danach einzeln aus dem Icon-Theme. Klicks neben die Karte gehen an
  die Fenster, die Tastatur bleibt beim Launcher.
- **Wallpaper-Picker** (Wallpaper-Binding): über dem abgedunkelten Monitor unter dem Zeiger die Bilder aus
  `wallpaper_directory` als Karussell schräg gestellter Kacheln, das gewählte breit in der Mitte, die Nachbarn als
  Streifen daneben; ein neues gleitet in 180 ms an seinen Platz. Er öffnet auf dem Bild in Gebrauch, das Feld filtert
  nach Dateinamen, Pfeile, Tab, Mausrad über den Kacheln oder ein Klick auf einen Nachbarn wählen (am letzten Bild ist
  Schluss), Enter oder ein Klick auf das gewählte übernimmt es, Escape oder ein Klick daneben schließen. Übernommen
  wird ein Bild erst, wenn es sich laden lässt; dann speichert er es als `file://`-Adresse in
  `$XDG_STATE_HOME/splice/wallpaper.json`, das Wallpaper wechselt sofort, und das Farbschema erzeugt
  `--theme-from-wallpaper` im Hintergrund. Jedes Öffnen liest Ordner und Auswahl neu und vergisst beim Schließen
  alles, wie der Quickshell-Picker, der nur bei Bedarf lief.
- **Sperrbildschirm** (Sperren-Binding, `--lock`, swayidle): Der Compositor sperrt selbst, statt `lock.qml`
  zu starten. Kein Client zeigt sich dahinter, bekommt eine Taste (außer den Programm-Bindings mit
  `allow_when_locked`), einen Klick oder das Mausrad, und Aufnahmen ganzer Monitore zeigen nur den
  Sperrbildschirm. Ein anderer
  Bildschirmsperrer wird abgewiesen (`finished`) und kann die Sperre weder übernehmen noch aufheben. Das Passwort
  prüft PAM (Dienst `splice-lock`, mit `$SPLICE_PAM_DIR` aus einem anderen Verzeichnis) mit
  `splice-shell-services --authenticate`, einem eigenen Prozess je Versuch: Stirbt ein PAM-Modul, endet nur dieser
  Versuch; hängt es, wartet der Sperrbildschirm wie `lock.qml`, bis es endet, und nimmt so lange keine Taste an.
  Entsperrt wird nur bei sauberem Ende mit `success` für den laufenden Versuch. Wie bei `lock.qml`
  heißt jedes andere Ergebnis außer „Zu viele Versuche“ „Falsches Passwort“ (Quickshell überschreibt seine
  Fehlermeldung sofort damit). Das Bild folgt `LoginScreen.qml`: auf jedem Monitor das Bild des Themes (`source`
  in `theme.json`, beim Sperren gelesen wie die Farben), in einem Worker-Thread auf den Monitor zugeschnitten und
  abgedunkelt, bis dahin die Hintergrundfarbe; auf dem Monitor mit Workspace 1 Uhr, Datum (ausgeschriebene Namen)
  und die Karte mit Initiale, `$USER`, Passwortfeld (Punkte, rot nach einem abgelehnten Passwort) und Knopf, der
  beim Überfahren aufleuchtet und wie Enter prüft. Das Feld kennt Tippen, Rücktaste und Enter.
- **Anmeldebildschirm** (`splice-compositor --greeter` ohne Befehl, im NixOS-Modul die Standardeinstellung von
  `programs.splice.greeter`): die Karte des Sperrbildschirms mit Feld für den
  Benutzer (`SPLICE_GREETER_USER` füllt es vor, dann hat das Passwort die Tastatur), unten links die Sitzungen aus
  `SPLICE_GREETER_SESSIONS` (nach Dateiname, erstes `Name=` und `Exec=` ohne Feldcodes; nur bei mehr als einer),
  unten rechts Neustart und Ausschalten (`systemctl reboot|poweroff`), als Hintergrund `SPLICE_GREETER_BACKGROUND`.
  Tab geht wie bei Qt durch Felder und Knöpfe, Enter im Benutzerfeld zum Passwort, Enter oder Leertaste auf einem
  Knopf drückt ihn. Er liegt von Anfang an hinter der eigenen Sperre: keine Bindings (außer Programm-Bindings mit
  `allow_when_locked`), kein Client erreichbar. Die Anmeldung läuft über greetd (`$GREETD_SOCK`) mit
  `splice-shell-services --login`, einem eigenen Prozess je Versuch, wie Quickshells `Greetd`: jede Frage bekommt
  das Passwort, Fehlermeldungen von PAM erscheinen, „Sitzung startet …“, dann endet der Compositor mit Code 0, und
  greetd startet die Sitzung. Eine liegengebliebene Sitzung („a session is already being configured“) bricht er ab
  und beginnt neu. Dienstprozess und Nachtlicht laufen hier nicht. `--greeter "<Befehl>"` startet wie bisher den
  Befehl (`greeter.qml`).
- **Nachtlicht**: steht in `$XDG_STATE_HOME/splice/night-light` „1“, läuft `gammastep -O 4000` wie bei der
  Quickshell-Bar (nur auf echter Hardware); umschalten lässt es sich erst mit dem Commandcenter.

## Was noch fehlt

In den Panels führt Tab noch nicht von Knopf zu Knopf
(bei Quickshell erreichte Tab jeden Knopf); im Kalenderformular wechselt Tab nur zwischen den vier Feldern. Die
Textfelder (Kalenderformular, Suche des Launchers, Filter des Pickers) kennen Tippen, Löschen, Pfeile, Pos1/Ende, Klicks und gehaltene
Tasten (mit `repeat_delay`/`repeat_rate` aus `[input]`), aber keine Auswahl und keine Zwischenablage, und der
Cursor blinkt nicht. Der Launcher sortiert Namen ohne Groß-/Kleinschreibung statt nach ICU-Sortierregeln wie
JavaScripts `localeCompare` und zeigt keine Scrollleiste. Die Kanten der Picker-Kacheln sind geglättet (bei Qt
treppig), über ihnen bleibt der Zeiger ein Pfeil (bei Qt eine Hand), und scheitert das Farbschema, steht das nur im
Log, weil der Picker dann schon zu ist. Datum und Uhrzeit
kennen nur deutsche und englische Namen. Links in Benachrichtigungen erscheinen unterstrichen in Textfarbe, nicht
wie bei Qt blau.

## Aufbau

- `crates/splice-ui` (ohne Compositor-Abhängigkeit, mit Tests): Szenen aus Rechtecken, Texten und Bögen
  (`bar`, `power`, `notices`), Zeichnen auf der CPU (tiny-skia, Text mit cosmic-text und den Schriften aus
  fontconfig), Theme, Texte, Uhr.
- `src/shell.rs`: füllt die Szenen aus dem Zustand des Compositors, zeichnet sie nur bei Änderungen neu
  (dieselben Änderungen, die das Bar-Protokoll meldet, dazu Hover, Uhr, Countdown) und legt die Bilder als
  Texturen in den Frame. Die Schriften laden beim Start in einem eigenen Thread.
- Reihenfolge von vorn nach hinten: Powermenü, `Overlay`-Layer, Hinweise, Vollbildfenster, `Top`-Layer,
  Bar, Fenster. Zeiger und Klicks folgen derselben Reihenfolge (`pointer_target_over` in `src/input.rs`).

- `splice-shell-services` (`crates/splice-shell-services`, eigenes Programm neben dem Compositor): fragt die
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
- `crates/splice-shell-protocol`: eine JSON-Zeile je Nachricht über stdin/stdout. Der Dienstprozess schickt bei
  jeder Änderung den ganzen Status und jede Benachrichtigung einzeln (Bilder als Base64), der Compositor Befehle. Der Compositor liest ohne zu blockieren, verwirft
  Befehle, die der Prozess nicht abnimmt, und startet ihn nach dem Ende mit wachsender Pause neu (1 bis 30
  Sekunden; nach einer Minute Laufzeit wieder kurz). Stirbt der Compositor, endet der Prozess mit ihm
  (`PR_SET_PDEATHSIG`). Statusänderungen protokolliert er mit `RUST_LOG=splice_compositor::shell_services=debug`.
  Eine Lautstärke, die die Bar verlangt hat, zeigt sie, bis ein Bericht sie bestätigt (höchstens eine Sekunde):
  Berichte von vor dem Befehl würden sonst zurückspringen lassen, und die nächste Raste zählte vom alten Wert.
- `crates/splice-ui/src/notifications.rs`: die Karte (`NotificationCard.qml`) und die Zentrale
  (`NotificationCenter.qml` im Panelrahmen von `bar.qml`); `src/shell_notifications.rs` hält Liste, Toasts und
  Zentrale. cosmic-text 0.19 lässt beim Kürzen der letzten erlaubten Zeile das Leerzeichen des Umbruchs vor ihr
  stehen; `text.rs` beginnt solche Folgezeilen beim ersten Wort (Workaround, bis cosmic-text das selbst richtig
  macht).

- `crates/splice-ui/src/calendar.rs`: Inhalt des Kalender-Panels (`CalendarPanelInteractive.qml`) mit den
  Formularfeldern; `src/shell_calendar.rs` ruft `splice-calendar` auf, hält Monat, Termine und Formular und nimmt die
  Tasten der Felder.

- `crates/splice-ui/src/launcher.rs` (Suche, Sortierung, Karte nach `shell.qml`), `src/shell_launcher.rs` (Öffnen,
  Tasten, Starten), `crates/splice-shell-services/src/apps.rs` (Desktop-Einträge nach Quickshells
  `desktopentry.cpp`). Hält die Shell die Tastatur, wiederholt der Compositor gehaltene Tasten selbst
  (`shell_key_pressed`), wie es Clients für sich tun.

- `crates/splice-ui/src/wallpaper_picker.rs` (Maße des Karussells nach `wallpaper.qml`, Texte, Treffer auf schrägen
  Kacheln), `src/shell_wallpaper.rs` (Ordner lesen, Bilder in Worker-Threads dekodieren, höchstens zwei zugleich, nur
  für die sichtbaren Kacheln und auf die Größe der breiten Kachel verkleinert, Speichern, Theme). Die Kacheln zeichnet
  die GPU: ein Shader (`tile` in `src/effects.rs`) richtet jeden Punkt wieder gerade und malt dort Fläche, Bild,
  Abdunklung und Rahmen. Auf der CPU kostete eine Kachel bei 2560×1440 um 14 ms, das Gleiten bräuchte das jeden
  Frame; so malt die CPU nur die Texte neu.

- `crates/splice-ui/src/lock_screen.rs` (Uhr und Karte), `src/shell_lock.rs` (Passwort, Prüfung, Hintergrund), `own_lock`/`own_unlock` in `src/session.rs` (eigene
  Sperrzustände neben denen von `ext-session-lock`), `crates/splice-shell-services/src/pam.rs` (die Prüfung wie
  Quickshells `PamContext`: nur `pam_authenticate`, jede Frage bekommt das Passwort).
- `src/shell_greeter.rs` (Sitzungen, Anmeldung, Knöpfe des Anmeldebildschirms), `crates/splice-shell-services/src/login.rs`
  (greetds IPC: JSON mit vorangestellter Länge).

## Test

`tests/builtin_shell_smoke.py`: Bar gezeichnet und Fenster darunter, Workspace-Knöpfe, Powermenü,
abgebrochener und abgelaufener Logout-Countdown, Konfigurationshinweis.
`tests/shell_services_smoke.py`: mit eigenem PipeWire (Null-Sink), nachgebautem Player (`tests/fake_mpris.py`) und
nachgebautem BlueZ (`tests/fake_bluez.py`): Mausrad und Rechtsklick auf der Lautstärke, Medientitel, Bluetooth,
Neustart des Dienstprozesses nach `kill -9`.
`tests/control_smoke.py`: nachgebautes BlueZ und Platzhalter für `gammastep`, `ip`, `wl-copy`, `splice-settings`:
Bluetooth an, verbinden, suchen, koppeln, Adresse kopieren, Nachtlicht an und aus, Einstellungen starten, gemerktes
Nachtlicht beim nächsten Start.
`tests/audio_smoke.py`: eigenes PipeWire mit zwei Ausgängen, Mikrofon, Smart-Filter und Anwendungsstrom: Regler
ziehen, Stummschalten, anderen Ausgang wählen. Die PipeWire-Tests starten WirePlumber ohne Hardware-Monitore
(`hermetic_wireplumber` in `tests/smoke_support.py`): sonst fände es die echten Geräte des Rechners, und ein Test,
der die Lautstärke des Standardausgangs setzt, könnte die eines echten Geräts setzen.
`tests/tray_smoke.py`: nachgebauter Tray-Eintrag (`tests/fake_tray.py`) mit Menü: Symbol in der Bar, Aktivieren,
zweite Aktivierung, Scrollen, Menü mit Untermenü und verstecktem Eintrag, Klick auf einen Eintrag, Eintrag endet.
`tests/media_smoke.py`: nachgebauter Player mit Cover, Medien-Panel mittig, Abspielen/Pause/Weiter und ein
nicht erlaubtes Zurück.
`tests/calendar_smoke.py`: Platzhalter für `splice-calendar` und `foot`: die Uhr öffnet das Panel (Termine des Monats
und Kalender abgefragt), ein Termin ohne Titel wird nicht angelegt, getippter Titel und gewählter Kalender gehen an
`splice-calendar create`, danach lädt es neu; Monat vor und zurück, Kalender-App öffnen schließt das Panel.
`tests/launcher_smoke.py`: Test-Apps wie in splice-shells Launcher-Test: Suche über Schlüsselwörter, Symbol aus
`Icon=`, Start mit Argumenten und Verzeichnis, Pfeil und Terminal-App, keine Treffer, gehaltene Rücktaste, Escape,
Klick auf eine Zeile.
`tests/wallpaper_picker_smoke.py`: Bilder wie in splice-shells Picker-Test (Leerzeichen, `#` und `%` im Namen) und
ein kaputtes: Filter und Enter speichern, Wallpaper und Theme folgen; Start auf dem Bild in Gebrauch, das kaputte wird
abgelehnt, keine Treffer, Escape, Klick daneben, Klick auf Nachbar und gewähltes, gehaltener Pfeil endet am letzten,
Mausrad.
`tests/lock_screen_smoke.py`: PAM mit eigenem Dienstverzeichnis (`pam_exec`, kein echtes Konto, wie splice-shells
Test von `lock.qml`): `--lock` bestätigt, Fenster und Ecken verdeckt (Aufnahme mit grim), Befehle abgewiesen; falsches
Passwort, eine Prüfung, die mittendrin stirbt, und ein fremder Sperrer (`splice-test-client lock`) entsperren nicht,
das Fenster bekommt keine Taste, der Rand des Felds wird rot; das richtige Passwort entsperrt mit einem Klick auf den
Knopf. Ein Fenster unter dem ruhenden Zeiger verliert ihn beim Sperren; das Bild eines erst danach geschriebenen
`theme.json` erscheint abgedunkelt.
`tests/greeter_smoke.py`: `--greeter` mit zwei Monitoren gegen nachgebautes greetd (`tests/fake_greetd.py` aus
splice-shell): Karte nur auf dem ersten, Bindings tun nichts, falsches Passwort (`create_session`, `response wrong`,
roter Rand), Tab und Enter wählen die zweite Sitzung, das richtige Passwort startet sie ohne Feldcodes, der
Compositor endet mit 0; „Ausschalten“ ruft `systemctl poweroff` (ein Platzhalter vorn im `PATH`).
`tests/notifications_smoke.py`: auf eigenem Session-Bus Toast mit Ablaufzeit und Festhalten, „×“, Aktion und
`CloseNotification` mit den passenden D-Bus-Signalen, Zentrale über die Glocke mit „Alle löschen“, Escape und
Klick daneben. Nested meldet winit Drücken und Loslassen einer X-Mausradtaste je als
eine Raste; ein Klick dort sind also zwei.
