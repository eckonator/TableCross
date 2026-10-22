//! Languages of the user interface.
//!
//! The English text in the code is the key: `tr("Settings")` gives "Einstellungen" in German.
//! Placeholders `{}` are filled in order by `trf`. A missing translation keeps the English text, and
//! a test fails.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::settings::LangSetting;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::De as u8 {
        Lang::De
    } else {
        Lang::En
    }
}

/// "System" follows the system language: German if it starts with "de", else English.
pub fn apply_setting(s: LangSetting) {
    set_lang(match s {
        LangSetting::De => Lang::De,
        LangSetting::En => Lang::En,
        LangSetting::System => {
            if sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("de")) {
                Lang::De
            } else {
                Lang::En
            }
        }
    });
}

/// Translates a fixed text.
pub fn tr(en: &'static str) -> &'static str {
    if !en.chars().any(char::is_alphabetic) {
        return en;
    }
    match lang() {
        Lang::En => en,
        Lang::De => de(en).unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                tracing::debug!("translation missing: {en:?}");
            }
            en
        }),
    }
}

/// Translates a text with `{}` placeholders and fills them in order.
pub fn trf(en: &'static str, args: &[&dyn Display]) -> String {
    let template = tr(en);
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(a) = args.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

fn de(en: &str) -> Option<&'static str> {
    Some(match en {
        "(this computer)" => "(dieser Computer)",
        "A program holds Secure Input (password field): typing does not reach other computers right now. The system helper solves this." => {
            "Ein Programm hält die sichere Eingabe (Passwortfeld): Tastatureingaben erreichen andere Computer gerade nicht. Der Systemhelfer löst das."
        }
        "A wrong code was entered ({} attempts left)" => "Ein falscher Code wurde eingegeben (noch {} Versuche)",
        "About" => "Über",
        "above this computer" => "über diesem Computer",
        "All keys up, mouse and keyboard back to this computer (Ctrl+Shift+Alt+Esc)" => {
            "Alle Tasten los, Maus und Tastatur zurück an diesen Computer (Strg+Umschalt+Alt+Esc)"
        }
        "another device answered at the known address" => "unter der bekannten Adresse antwortete ein anderes Gerät",
        "Appearance" => "Aussehen",
        "Arrange" => "Anordnen",
        "Arrangement" => "Anordnung",
        "Arrange the computers like they stand on your desk." => {
            "Ordne die Computer so an, wie sie auf deinem Schreibtisch stehen."
        }
        "Desk" => "Schreibtisch",
        "Drag the computers to where they stand on your desk, in their real size. Where two touch, the pointer crosses (green); while dragging, the screens show it at their edges as well. Drop a computer onto another one if both are used in turn at the same place." => {
            "Zieh die Computer dorthin, wo sie auf deinem Schreibtisch stehen, in ihrer echten Größe. Wo sich zwei berühren, wechselt der Zeiger (grün); beim Ziehen zeigen das auch die Bildschirme an ihren Kanten. Zieh einen Computer auf einen anderen, wenn beide abwechselnd am selben Platz stehen."
        }
        "Take over the current size" => "Aktuelle Größe übernehmen",
        "Takes the screens it has now; each computer also does this by itself when its screens change." => {
            "Übernimmt die Bildschirme, die er gerade hat; das macht jeder Computer auch selbst, wenn sich seine Bildschirme ändern."
        }
        "A screen of this computer reports a size that cannot be right. Without a diagonal typed in it counts as large as a screen with the same resolution on another computer." => {
            "Ein Bildschirm dieses Computers meldet eine Größe, die nicht stimmen kann. Ohne eingetragene Diagonale zählt er so groß wie ein Bildschirm mit derselben Auflösung an einem anderen Computer."
        }
        "Screens of this computer" => "Bildschirme dieses Computers",
        "This system does not tell the size of its screens." => {
            "Dieses System verrät die Größe seiner Bildschirme nicht."
        }
        "Screen {}: {} × {} points" => "Bildschirm {}: {} × {} Punkte",
        "diagonal by hand:" => "Diagonale von Hand:",
        "reports {}" => "meldet {}",
        "reports no size" => "meldet keine Größe",
        "reports {}, which cannot be right" => "meldet {}, das kann nicht stimmen",
        "Done" => "Fertig",
        "Screen sizes …" => "Bildschirmgrößen …",
        "Arrange screens …" => "Bildschirme anordnen …",
        "Opens the display settings of the system: where the screens of this computer lie there is where the pointer moves between them, and the desk shows it a moment later." => {
            "Öffnet die Anzeige-Einstellungen des Systems: Wie die Bildschirme dieses Computers dort liegen, so wechselt der Zeiger zwischen ihnen, und der Schreibtisch zeigt es kurz darauf."
        }
        " inches" => " Zoll",
        "No computer is paired yet." => "Noch kein Computer gekoppelt.",
        "Pair new computers and manage the paired ones." => "Neue Computer koppeln und gekoppelte verwalten.",
        "Pairing" => "Kopplung",
        "Take off the desk" => "Vom Schreibtisch nehmen",
        "no neighbor" => "ohne Nachbarn",
        "Author" => "Autor",
        "Back" => "Zurück",
        "below this computer" => "unter diesem Computer",
        "Cancel" => "Abbrechen",
        "Clear list" => "Liste leeren",
        "cannot save the pairing ({})" => "Kopplung lässt sich nicht speichern ({})",
        "captured" => "erfasst",
        "Check again" => "Erneut prüfen",
        "Clipboard" => "Zwischenablage",
        "Clipboard taken over from {}" => "Zwischenablage von {} übernommen",
        "Code" => "Code",
        "Connected at {} · round trip {} · lost {} of {} · reconnects {}" => {
            "Verbunden über {} · Laufzeit {} · verloren {} von {} · Neuverbindungen {}"
        }
        "Connected to {}" => "Verbunden mit {}",
        "Connection to {} ended: {}" => "Verbindung zu {} beendet: {}",
        "Controlled by {}" => "Gesteuert von {}",
        "Controlling {}" => "Steuert {}",
        "Copy diagnostics" => "Diagnose kopieren",
        "Could not change the start with the system: {}" => "Der Start mit dem System ließ sich nicht ändern: {}",
        "Ctrl + Shift + Alt + arrow key" => "Strg + Umschalt + Alt + Pfeiltaste",
        "Ctrl + Shift + Alt + Esc" => "Strg + Umschalt + Alt + Esc",
        "Copied content stays on this computer: {} MB is more than allowed" => {
            "Kopierter Inhalt bleibt auf diesem Computer: {} MB sind mehr als erlaubt"
        }
        "Copied password stays on this computer (marked as private)" => {
            "Kopiertes Passwort bleibt auf diesem Computer (als privat markiert)"
        }
        "Dark" => "Dunkel",
        "Diagnostics copied to the clipboard." => "Diagnose in die Zwischenablage kopiert.",
        "Done." => "Erledigt.",
        "Drag each computer to the side where it stands on your desk." => {
            "Zieh jeden Computer auf die Seite, auf der er auf deinem Schreibtisch steht."
        }
        "Enter a code from another computer" => "Code eines anderen Computers eingeben",
        "Everything TableCross needs is in place." => "Alles, was TableCross braucht, ist eingerichtet.",
        "Everything was released" => "Alles wurde losgelassen",
        "Files" => "Dateien",
        "Find computers on the network automatically" => "Computer im Netz automatisch finden",
        "Fingerprint" => "Fingerabdruck",
        "Free software under the GNU General Public License, version 3 or later. It comes without any warranty." => {
            "Freie Software unter der GNU General Public License, Version 3 oder neuer. Ohne jede Gewährleistung."
        }
        "General" => "Allgemein",
        "Pointer & keyboard" => "Zeiger & Tastatur",
        "Go there" => "Dorthin wechseln",
        "Go to the computer on that side" => "Zum Computer auf dieser Seite wechseln",
        "Install system helper …" => "Systemhelfer installieren …",
        "Keyboard shortcuts" => "Tastenkürzel",
        "Largest content" => "Größter Inhalt",
        "Language" => "Sprache",
        "left of this computer" => "links von diesem Computer",
        "License" => "Lizenz",
        "Light" => "Hell",
        "Like the system" => "Wie das System",
        "Mouse and keyboard" => "Maus und Tastatur",
        "Mouse and keyboard are not captured right now." => "Maus und Tastatur werden gerade nicht erfasst.",
        "Mouse and keyboard cannot be captured: {}" => "Maus und Tastatur lassen sich nicht erfassen: {}",
        "Mouse and keyboard now control {}" => "Maus und Tastatur steuern jetzt {}",
        "Mouse and keyboard work on this computer" => "Maus und Tastatur arbeiten an diesem Computer",
        "Name" => "Name",
        "Name of this computer" => "Name dieses Computers",
        "Network" => "Netzwerk",
        "network error ({})" => "Netzwerkfehler ({})",
        "Next" => "Weiter",
        "no answer for 2 seconds" => "2 Sekunden keine Antwort",
        "No computer on the network shows a code right now." => "Gerade zeigt kein Computer im Netz einen Code.",
        "none" => "keiner",
        "None yet. Pair a computer on the page “Computers”." => {
            "Noch keiner. Kopple einen Computer auf der Seite „Computer“."
        }
        "not available: {}" => "nicht verfügbar: {}",
        "Not connected (searching …)" => "Nicht verbunden (suche …)",
        "Not connected: {}" => "Nicht verbunden: {}",
        "Not found? Enter the address" => "Nicht gefunden? Adresse eingeben",
        "not paired with this computer" => "nicht mit diesem Computer gekoppelt",
        "not placed" => "nicht platziert",
        "Not placed:" => "Nicht platziert:",
        "Whole desk" => "Ganzer Schreibtisch",
        "Update together with paired computers" => "Mit gekoppelten Computern aktualisieren",
        "When a paired computer runs a newer version, this one fetches the same version from GitHub and installs it." => {
            "Hat ein gekoppelter Computer eine neuere Version, holt dieser Computer dieselbe Version von GitHub und installiert sie."
        }
        "Check for updates" => "Nach Updates suchen",
        "TableCross {} is up to date." => "TableCross {} ist aktuell.",
        "Neither curl nor wget is installed, which the update check needs (Linux: sudo apt install curl)." => {
            "Weder curl noch wget ist installiert, die Update-Suche braucht eines davon (Linux: sudo apt install curl)."
        }
        "GitHub could not be reached; try again later." => {
            "GitHub war nicht erreichbar; bitte später noch einmal versuchen."
        }
        "used in turn" => "abwechselnd hier",
        "Update to {}" => "Auf {} aktualisieren",
        "Updating …" => "Wird aktualisiert …",
        "Remove TableCross" => "TableCross entfernen",
        "Remove TableCross …" => "TableCross entfernen …",
        "Removes TableCross from this computer with everything it set up: the system helper, the permissions, starting with the system and the program itself." => {
            "Entfernt TableCross von diesem Computer mit allem, was es eingerichtet hat: Systemhelfer, Berechtigungen, Start mit dem System und das Programm selbst."
        }
        "Also delete settings and pairings (the other computers forget this one)" => {
            "Auch Einstellungen und Kopplungen löschen (die anderen Computer vergessen diesen)"
        }
        "Removing TableCross …" => "TableCross wird entfernt …",
        "Really remove TableCross?" => "TableCross wirklich entfernen?",
        "Yes, remove" => "Ja, entfernen",
        "All screens were locked" => "Alle Bildschirme wurden gesperrt",
        "Lock all screens" => "Alle Bildschirme sperren",
        "Other computers …" => "Andere Computer …",
        "All computers …" => "Alle Computer …",
        "Log out on all computers?" => "Alle Computer abmelden?",
        "Log out on the other computers?" => "Die anderen Computer abmelden?",
        "Log out" => "Abmelden",
        "This computer logs out" => "Dieser Computer meldet sich ab",
        "Logging out: {}" => "Wird abgemeldet: {}",
        "Update first to log out: {}" => "Zum Abmelden erst aktualisieren: {}",
        "Shut down all computers?" => "Alle Computer herunterfahren?",
        "Shut down the other computers?" => "Die anderen Computer herunterfahren?",
        "No other computer is connected." => "Kein anderer Computer ist verbunden.",
        "Only this computer is connected." => "Nur dieser Computer ist verbunden.",
        "This computer and {}" => "Dieser Computer und {}",
        "Programs with unsaved documents may still ask before they quit." => {
            "Programme mit ungesicherten Dokumenten können vor dem Beenden noch nachfragen."
        }
        "Shut down" => "Herunterfahren",
        "This computer is shutting down" => "Dieser Computer wird heruntergefahren",
        "Shutting down: {}" => "Wird heruntergefahren: {}",
        "Ctrl + Shift + Alt + L" => "Strg + Umschalt + Alt + L",
        "Ctrl + Shift + Alt + 1 … 9" => "Strg + Umschalt + Alt + 1 … 9",
        "Go to the computer with that number (see Arrangement)" => {
            "Zum Computer mit dieser Nummer wechseln (siehe Anordnung)"
        }
        "Lock the screens of all computers" => "Die Bildschirme aller Computer sperren",
        "The button opens the settings and a Finder window with the helper file: switch it on there, or drag the file into the list." => {
            "Der Knopf öffnet die Einstellungen und ein Finder-Fenster mit der Datei des Systemhelfers: dort einschalten oder die Datei in die Liste ziehen."
        }
        "Test build {} ({})" => "Testversion {} ({})",
        "Update system helper …" => "Systemhelfer aktualisieren …",
        "Already switched on and still shown here (often after an update)? Select {} in the list, remove it with “–” and add it again with “+”." => {
            "Schon eingeschaltet und trotzdem hier aufgeführt (oft nach einem Update)? {} in der Liste auswählen, mit „–“ entfernen und mit „+“ neu hinzufügen."
        }
        "Update now" => "Jetzt aktualisieren",
        "Downloading …" => "Wird geladen …",
        "Installing, TableCross starts again by itself …" => {
            "Wird installiert, TableCross startet danach von selbst …"
        }
        "The update did not work: {}" => "Die Aktualisierung hat nicht geklappt: {}",
        "version {}" => "Version {}",
        "This computer runs version {}. Install the same version everywhere." => {
            "Dieser Computer hat Version {}. Installiere überall dieselbe Version."
        }
        "The arrangement is shared with all paired computers, so every keyboard reaches every screen." => {
            "Die Anordnung gilt für alle gekoppelten Computer, so erreicht jede Tastatur jeden Bildschirm."
        }
        "not paired here" => "nicht gekoppelt",
        "To control {} with this keyboard, pair it with this computer as well." => {
            "Um {} mit dieser Tastatur zu steuern, kopple ihn auch mit diesem Computer."
        }
        "Nothing yet." => "Noch nichts.",
        "now" => "jetzt",
        "off" => "aus",
        "Open Accessibility settings" => "Einstellungen für Bedienungshilfen öffnen",
        "Open Input Monitoring settings" => "Einstellungen für Eingabeüberwachung öffnen",
        "Open settings folder" => "Einstellungsordner öffnen",
        "Pair" => "Koppeln",
        "Pair a computer" => "Computer koppeln",
        "Paired computers" => "Gekoppelte Computer",
        "Paired with {}" => "Gekoppelt mit {}",
        "Paired: {}" => "Gekoppelt: {}",
        "Pairing failed: {}" => "Koppeln fehlgeschlagen: {}",
        "pairing removed" => "Kopplung entfernt",
        "Pairing with {} removed" => "Kopplung mit {} entfernt",
        "Permissions" => "Berechtigungen",
        "Port" => "Port",
        "Privacy" => "Datenschutz",
        "protocol error ({})" => "Protokollfehler ({})",
        "Quit TableCross" => "TableCross beenden",
        "Really remove?" => "Wirklich entfernen?",
        "Recent messages" => "Letzte Meldungen",
        "Release everything" => "Alles loslassen",
        "Release everything: all keys up, mouse and keyboard back to this computer" => {
            "Alles loslassen: alle Tasten los, Maus und Tastatur zurück an diesen Computer"
        }
        "Remove" => "Entfernen",
        "Remove pairing …" => "Kopplung entfernen …",
        "replaced by a newer connection" => "durch eine neuere Verbindung ersetzt",
        "Restart now" => "Jetzt neu starten",
        "right of this computer" => "rechts von diesem Computer",
        "Screens" => "Bildschirme",
        "Search" => "Suche",
        "Settings" => "Einstellungen",
        "Setup …" => "Einrichtung …",
        "Shares one mouse, keyboard and clipboard between the computers on your desk, on the local network only." => {
            "Teilt eine Maus, eine Tastatur und die Zwischenablage zwischen den Computern auf deinem Schreibtisch, nur im lokalen Netz."
        }
        "Show a code here" => "Hier einen Code zeigen",
        "Show log file" => "Logdatei zeigen",
        "Troubleshooting" => "Fehlersuche",
        "Top and bottom" => "Oben und unten",
        "switch at once" => "sofort wechseln",
        "after a moment of pushing" => "nach kurzem Drücken",
        "after touching twice" => "nach zweimal Anstoßen",
        "The Dock and the menu bar are at these edges: switching only after pushing or a double tap keeps them from switching by accident. Left and right always switch at once." => {
            "An diesen Kanten liegen Dock und Menüleiste: Wechseln erst nach Drücken oder zweimal Anstoßen verhindert versehentliche Wechsel. Links und rechts wechselt der Zeiger immer sofort."
        }
        "Corners" => "Ecken",
        "no switching this close to a corner" => "so nah an einer Ecke kein Wechsel",
        "At left and right edges, so the pointer can reach things in the corners (0: off)." => {
            "An den linken und rechten Kanten, damit der Zeiger Dinge in den Ecken erreicht (0: aus)."
        }
        "Offline computers" => "Computer offline",
        "Skip computers that are offline" => "Computer überspringen, die offline sind",
        "AirDrop radio" => "AirDrop-Funk",
        "Crossings" => "Übergänge",
        "Keyboard layout" => "Tastaturlayout",
        "Login screen" => "Anmeldebildschirm",
        "Can be controlled on the login screen" => "Auch am Anmeldebildschirm steuerbar",
        "Before anyone logs in, this Mac can be used from the other computers as well, for example to type the password. Needs the system helper, and tablecross-loginwindow needs the Accessibility permission (the status page shows how)." => {
            "Bevor sich jemand anmeldet, lässt sich dieser Mac ebenfalls von den anderen Computern aus bedienen, etwa um das Passwort einzutippen. Braucht den Systemhelfer, und tablecross-loginwindow braucht das Recht „Bedienungshilfen“ (die Statusseite zeigt, wie)."
        }
        "To be controlled on the login screen, tablecross-loginwindow needs the Accessibility permission (in newer macOS versions “Device control and data access”)." => {
            "Damit dieser Mac am Anmeldebildschirm steuerbar ist, braucht tablecross-loginwindow das Recht „Bedienungshilfen“ (in neueren macOS-Versionen „Gerätesteuerung und Datenzugriff“)."
        }
        "The button opens the settings and a Finder window with the file: drag it into the list and switch it on." => {
            "Der Knopf öffnet die Einstellungen und ein Finder-Fenster mit der Datei: Datei in die Liste ziehen und einschalten."
        }
        "Use the layout of the computer in control" => "Layout des steuernden Computers verwenden",
        "While another computer controls this one, its keyboard layout (German, Swiss German, US …) is switched on here, so every key gives the same character. The layout used before comes back afterwards." => {
            "Während ein anderer Computer diesen steuert, wird hier dessen Tastaturlayout (Deutsch, Schweizerdeutsch, US …) eingestellt, damit jede Taste dasselbe Zeichen ergibt. Danach kommt das vorherige Layout zurück."
        }
        "{} types with the keyboard layout {}, which is not set up here (add it in the system settings)" => {
            "{} schreibt mit dem Tastaturlayout {}, das hier nicht eingerichtet ist (in den Systemeinstellungen hinzufügen)"
        }
        "Always show where the pointer crosses" => "Immer zeigen, wo der Zeiger hinüberwechselt",
        "Thin green bars at the edges of the screens where the pointer goes to another computer. Otherwise they appear only while the arrangement is changed." => {
            "Schmale grüne Balken an den Bildschirmrändern, wo der Zeiger zu einem anderen Computer wechselt. Sonst erscheinen sie nur, während die Anordnung geändert wird."
        }
        "Scroll the other way" => "Andersherum scrollen",
        "While controlling this computer from here, the wheel scrolls the other way round" => {
            "Wenn dieser Computer von hier aus gesteuert wird, scrollt das Rad andersherum"
        }
        "Scroll speed" => "Scrolltempo",
        "While controlling this computer from here (1: as here)" => {
            "Wenn dieser Computer von hier aus gesteuert wird (1: wie hier)"
        }
        "Pause AirDrop and Handoff while sharing" => "AirDrop und Handoff beim Teilen pausieren",
        "Their radio link (AWDL) leaves the Wi-Fi channel every few seconds, and the pointer stutters meanwhile. The system helper pauses it while this Mac controls or is controlled, and lets it run again afterwards. AirDrop, Handoff and AirPlay to nearby devices wait until then." => {
            "Ihre Funkverbindung (AWDL) verlässt alle paar Sekunden den WLAN-Kanal, und der Zeiger ruckelt so lange. Der Systemhelfer pausiert sie, solange dieser Mac steuert oder gesteuert wird, und lässt sie danach wieder laufen. AirDrop, Handoff und AirPlay zu Geräten in der Nähe warten so lange."
        }
        "Accessibility is granted now. Please quit TableCross and start it again." => {
            "Die Bedienungshilfen sind jetzt erlaubt. Bitte TableCross beenden und neu starten."
        }
        "When a computer in the arrangement is off or not connected, the pointer goes on to the next computer behind it in the same direction." => {
            "Ist ein Computer der Anordnung aus oder nicht verbunden, geht der Zeiger in derselben Richtung weiter zum nächsten Computer dahinter."
        }
        "Ctrl + Shift + Alt + F" => "Strg + Umschalt + Alt + F",
        "Keep the pointer on this computer, or let it go again" => {
            "Zeiger auf diesem Computer festhalten oder wieder freigeben"
        }
        "Keep the pointer here" => "Zeiger hier festhalten",
        "Pointer kept here ✕" => "Zeiger festgehalten ✕",
        "The pointer does not cross edges (Ctrl+Shift+Alt+F). Click to let it go." => {
            "Der Zeiger wechselt über keine Kante (Strg+Umschalt+Alt+F). Klicken zum Freigeben."
        }
        "Sleep" => "Ruhezustand",
        "Keep this computer from going to sleep" => "Diesen Computer nicht in den Ruhezustand gehen lassen",
        "While TableCross runs. A computer that sleeps drops out, and with it the way over it to the computers behind." => {
            "Solange TableCross läuft. Ein schlafender Computer fällt weg und mit ihm der Weg über ihn zu den Computern dahinter."
        }
        "Keep the screen on as well" => "Auch den Bildschirm anlassen",
        "This computer cannot be controlled from the others: the system did not allow remote control. Restart TableCross and allow it when asked." => {
            "Dieser Computer kann von den anderen nicht gesteuert werden: Das System hat die Fernsteuerung nicht erlaubt. TableCross neu starten und die Erlaubnis bestätigen."
        }
        "Add an address …" => "Adresse hinzufügen …",
        "Where to reach this computer when it is not found on the network by itself" => {
            "Wo dieser Computer zu erreichen ist, wenn er im Netz nicht von selbst gefunden wird"
        }
        "Add" => "Hinzufügen",
        "Universal Control" => "Universelle Steuerung",
        "Switch off Universal Control of macOS" => "Universelle Steuerung von macOS ausschalten",
        "Switch off Universal Control" => "Universelle Steuerung ausschalten",
        "Handoff" => "Handoff",
        "Switch off Handoff of macOS" => "Handoff von macOS ausschalten",
        "Could not switch off Handoff: {}" => "Handoff ließ sich nicht ausschalten: {}",
        "With Handoff, macOS carries the clipboard to your other Macs as well and sends what TableCross shares back as new: pasting then waits for the other Mac. TableCross switches it off at start. This also stops the clipboard with an iPhone or iPad." => {
            "Mit Handoff bringt macOS die Zwischenablage ebenfalls zu deinen anderen Macs und schickt, was TableCross teilt, als neu zurück: Einfügen wartet dann auf den anderen Mac. TableCross schaltet es beim Start aus. Damit endet auch die Zwischenablage mit iPhone oder iPad."
        }
        "Could not switch off Universal Control: {}" => "Universelle Steuerung ließ sich nicht ausschalten: {}",
        "Universal Control also moves the pointer to other Macs at the screen edges. TableCross switches it off at start so the two do not fight over the pointer." => {
            "Die Universelle Steuerung schiebt den Zeiger an den Bildschirmrändern ebenfalls zu anderen Macs. TableCross schaltet sie beim Start aus, damit sich beide nicht um den Zeiger streiten."
        }
        "Universal Control of macOS is switched on. It also moves the pointer to other Macs at the screen edges, gets in the way of TableCross and takes the pointer over even when TableCross is quit." => {
            "Die Universelle Steuerung von macOS ist eingeschaltet. Sie schiebt den Zeiger an den Bildschirmrändern ebenfalls zu anderen Macs, kommt TableCross in die Quere und übernimmt den Zeiger sogar, wenn TableCross beendet ist."
        }
        "Collect the logs of the other computers here" => "Protokolle der anderen Computer hier sammeln",
        "The connected computers send their log to this one while it is on, so a problem can be looked at in one place. Only on paired computers, over the encrypted connection; the logs contain no typed text." => {
            "Solange das an ist, schicken die verbundenen Computer ihr Protokoll an diesen, damit sich ein Problem an einer Stelle untersuchen lässt. Nur unter gekoppelten Computern und über die verschlüsselte Verbindung; die Protokolle enthalten keinen getippten Text."
        }
        "Open the folder with the collected logs" => "Ordner mit den gesammelten Protokollen öffnen",
        "Show pairing code" => "Kopplungscode zeigen",
        "Share the clipboard with the other computers" => "Zwischenablage mit den anderen Computern teilen",
        "Show TableCross" => "TableCross zeigen",
        "Skip setup" => "Einrichtung überspringen",
        "Source code" => "Quellcode",
        "Start" => "Start",
        "Start TableCross on the other computer as well. Show a code on one of them and type it on the other." => {
            "Starte TableCross auch auf dem anderen Computer. Lass dir auf einem einen Code zeigen und tippe ihn auf dem anderen ein."
        }
        "Start TableCross with the system" => "TableCross mit dem System starten",
        "Status" => "Status",
        "Stop" => "Beenden",
        "Swap ⌘ and Ctrl" => "⌘ und Strg tauschen",
        "TableCross could not start" => "TableCross konnte nicht starten",
        "TableCross is shutting down" => "TableCross wird beendet",
        "TableCross needs a few things before it can share mouse and keyboard:" => {
            "TableCross braucht noch ein paar Dinge, bevor es Maus und Tastatur teilen kann:"
        }
        "TableCross needs the Accessibility permission to use mouse and keyboard." => {
            "TableCross braucht die Berechtigung „Bedienungshilfen“, um Maus und Tastatur zu nutzen."
        }
        "TableCross talks only to the computers you paired, encrypted, on your local network. No account, no cloud, no telemetry. If you allow it, it asks GitHub for new versions." => {
            "TableCross spricht nur mit den Computern, die du gekoppelt hast, verschlüsselt und nur im lokalen Netz. Kein Konto, keine Cloud, keine Telemetrie. Wenn du es erlaubst, fragt es bei GitHub nach neuen Versionen."
        }
        "Takes effect after a restart of the connection." => "Wirkt nach einem Neustart der Verbindung.",
        "That did not work: {}" => "Das hat nicht geklappt: {}",
        "the other computer cannot be reached" => "der andere Computer ist nicht erreichbar",
        "the other computer cannot be reached: its address {} is not in this computer's network (a virtual machine needs a bridged network adapter instead of NAT)" => {
            "der andere Computer ist nicht erreichbar: seine Adresse {} liegt nicht im Netz dieses Computers (eine virtuelle Maschine braucht eine Netzwerkbrücke statt NAT)"
        }
        "the other computer is not showing a code (expired or too many attempts)" => {
            "der andere Computer zeigt keinen Code (abgelaufen oder zu viele Versuche)"
        }
        "the other computer quit TableCross" => "der andere Computer hat TableCross beendet",
        "the other computer uses version {} of the protocol, this one {}: please update the older one" => {
            "der andere Computer nutzt Version {} des Protokolls, dieser {}: bitte den älteren aktualisieren"
        }
        "the other computer was not found on the network" => "der andere Computer wurde im Netz nicht gefunden",
        "the other computer went to sleep" => "der andere Computer ist in den Ruhezustand gegangen",
        "Text, formatted text and images. Passwords copied from a password manager stay on this computer." => {
            "Text, formatierter Text und Bilder. Aus einem Passwortmanager kopierte Passwörter bleiben auf diesem Computer."
        }
        "The pairing code is no longer valid after too many attempts" => {
            "Der Kopplungscode gilt nach zu vielen Versuchen nicht mehr"
        }
        "The system helper could not take {} keyboard(s); their keys also reach this Mac." => {
            "Der Systemhelfer konnte {} Tastatur(en) nicht übernehmen; ihre Tasten kommen auch an diesem Mac an."
        }
        "The system helper is not installed. Without it, typing on other computers stops while a password field is active on this Mac." => {
            "Der Systemhelfer ist nicht installiert. Ohne ihn kommen Tastatureingaben auf anderen Computern nicht an, solange an diesem Mac ein Passwortfeld aktiv ist."
        }
        "The system helper is older than the app; please update it." => {
            "Der Systemhelfer ist älter als die App; bitte aktualisieren."
        }
        "The system helper still needs the Input Monitoring permission." => {
            "Der Systemhelfer braucht noch die Berechtigung „Eingabeüberwachung“."
        }
        "Then type it on the other computer." => "Dann am anderen Computer eintippen.",
        "This computer" => "Dieser Computer",
        "this computer was asleep" => "dieser Computer war im Ruhezustand",
        "UDP port" => "UDP-Port",
        "unknown" => "unbekannt",
        "valid for {} min" => "gültig für {} min",
        "Version {} ({})" => "Version {} ({})",
        "While controlling this computer from here, ⌘ acts as Ctrl and the other way round" => {
            "Solange dieser Computer von hier aus gesteuert wird, wirkt ⌘ wie Strg und umgekehrt"
        }
        "Windows switched off the input capture {} times; it was restarted." => {
            "Windows hat die Eingabeerfassung {}-mal abgeschaltet; sie wurde neu gestartet."
        }
        "Woke from sleep, connecting again" => "Aus dem Ruhezustand aufgewacht, verbinde neu",
        "wrong code" => "falscher Code",
        "{} controls this computer" => "{} steuert diesen Computer",
        "{} h ago" => "vor {} h",
        "{} min ago" => "vor {} min",
        "{} removed the pairing" => "{} hat die Kopplung entfernt",
        "{} s ago" => "vor {} s",
        "{} was refused: {}" => "{} wurde abgewiesen: {}",
        "Copied on {}: {}." => "Auf {} kopiert: {}.",
        "Copied on {}: {}. Paste them where you want them, with ⌘V in the Finder." => {
            "Auf {} kopiert: {}. Mit ⌘V im Finder dort einsetzen, wo du sie haben möchtest."
        }
        "Save files …" => "Dateien speichern …",
        "This desktop shows no status icons, so the TableCross symbol and its menu are missing. On GNOME, install the AppIndicator extension, log out and in again and switch it on in the app “Extensions” (on Debian and Ubuntu:" => {
            "Diese Oberfläche zeigt keine Symbole in der Leiste, deshalb fehlen das TableCross-Symbol und sein Menü. Unter GNOME die Erweiterung AppIndicator installieren, ab- und wieder anmelden und sie in der App „Erweiterungen“ einschalten (unter Debian und Ubuntu:"
        }
        "Copy command" => "Befehl kopieren",
        "Command copied to the clipboard." => "Befehl in die Zwischenablage kopiert.",
        "Save files from the clipboard …" => "Dateien aus der Zwischenablage speichern …",
        "Save files from the clipboard" => "Dateien aus der Zwischenablage speichern",
        "No files are copied on another computer." => "Auf keinem anderen Computer sind Dateien kopiert.",
        "Copied on {}:" => "Auf {} kopiert:",
        "{} files, {}" => "{} Dateien, {}",
        "Save in" => "Speichern in",
        "Change …" => "Ändern …",
        "Save" => "Speichern",
        "from {}" => "von {}",
        "to {}" => "an {}",
        "{} file(s), done" => "{} Datei(en), fertig",
        "Open folder" => "Ordner öffnen",
        "cancelled" => "abgebrochen",
        "failed: {}" => "fehlgeschlagen: {}",
        "{} files from {} are in {} and on the clipboard" => "{} Dateien von {} liegen in {} und in der Zwischenablage",
        "{} files from {} are in {}" => "{} Dateien von {} liegen in {}",
        "File transfer with {} failed: {}" => "Dateiübertragung mit {} fehlgeschlagen: {}",
        "A new TableCross is available ({})." => "Eine neue Version von TableCross ist da ({}).",
        "Download" => "Herunterladen",
        "New versions" => "Neue Versionen",
        "May TableCross look for new versions on GitHub now and then? Only the question for the newest release is sent, nothing about you or your computers." => {
            "Darf TableCross ab und zu bei GitHub nach neuen Versionen schauen? Gesendet wird nur die Frage nach der neuesten Version, nichts über dich oder deine Computer."
        }
        "Yes, look for new versions" => "Ja, nach neuen Versionen schauen",
        "No" => "Nein",
        "Updates" => "Updates",
        "Look for new versions on GitHub" => "Bei GitHub nach neuen Versionen schauen",
        "About TableCross" => "Über TableCross",
        "How TableCross behaves on this computer." => "Wie sich TableCross auf diesem Computer verhält.",
        "ID {}" => "ID {}",
        "Setup" => "Einrichtung",
        "This computer, its connections and what happened recently." => {
            "Dieser Computer, seine Verbindungen und was zuletzt passiert ist."
        }
        "Three short steps, then TableCross is ready." => "Drei kurze Schritte, dann ist TableCross bereit.",
        "Working on this computer" => "Arbeitet an diesem Computer",
        "connected" => "verbunden",
        "mouse and keyboard captured" => "Maus und Tastatur erfasst",
        "mouse and keyboard not available" => "Maus und Tastatur nicht verfügbar",
        "mouse and keyboard off" => "Maus und Tastatur aus",
        "not connected" => "nicht verbunden",
        "this computer" => "dieser Computer",
        "{} screen(s)" => "{} Bildschirm(e)",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// String literals of all `tr("…")` and `trf("…", …)` calls in the source code (also with the
    /// text on the next line).
    fn literal_keys(src: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for call in ["tr(", "trf("] {
            let mut rest = src;
            while let Some(i) = rest.find(call) {
                let before = rest[..i].chars().next_back();
                rest = &rest[i + call.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let trimmed = rest.trim_start();
                let Some(literal) = trimmed.strip_prefix('"') else {
                    continue;
                };
                rest = literal;
                let mut key = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => key.push('\n'),
                            Some('t') => key.push('\t'),
                            // Line continuation (also with Windows line endings): leading spaces of the
                            // next line are dropped
                            Some('\n' | '\r') => {
                                let tail = chars.as_str().trim_start();
                                chars = tail.chars();
                            }
                            Some(other) => key.push(other),
                            None => break,
                        },
                        c => key.push(c),
                    }
                }
                keys.push(key);
            }
        }
        keys
    }

    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Every fixed text in the interface has a German translation.
    #[test]
    fn every_literal_ui_text_has_a_german_translation() {
        let mut files = Vec::new();
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut missing = Vec::new();
        let mut found = 0;
        for path in files {
            if path.ends_with("i18n.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for key in literal_keys(&src) {
                found += 1;
                if key.chars().any(char::is_alphabetic) && de(&key).is_none() {
                    missing.push(key);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(found > 50, "too few texts found ({found}), is the scan broken?");
        assert!(
            missing.is_empty(),
            "missing German translations:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn scanner_reads_continued_lines_with_any_line_ending() {
        for newline in ["\n", "\r\n"] {
            let src = format!("tr(\"first \\{newline}         second\")");
            assert_eq!(literal_keys(&src), ["first second"], "{newline:?}");
        }
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        set_lang(Lang::En);
        assert_eq!(trf("{} s ago", &[&5]), "5 s ago");
        set_lang(Lang::De);
        assert_eq!(trf("{} s ago", &[&5]), "vor 5 s");
        assert_eq!(tr("not in the table"), "not in the table");
        set_lang(Lang::En);
    }
}
