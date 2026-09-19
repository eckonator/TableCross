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
        "(this computer)" => "(dieser Rechner)",
        "A program holds Secure Input (password field): typing does not reach other computers right now. The system helper solves this." => {
            "Ein Programm hält die sichere Eingabe (Passwortfeld): Tastatureingaben erreichen andere Rechner gerade nicht. Der Systemhelfer löst das."
        }
        "A wrong code was entered ({} attempts left)" => "Ein falscher Code wurde eingegeben (noch {} Versuche)",
        "About" => "Über",
        "above this computer" => "über diesem Rechner",
        "All keys up, mouse and keyboard back to this computer (Ctrl+Shift+Alt+Esc)" => {
            "Alle Tasten los, Maus und Tastatur zurück an diesen Rechner (Strg+Umschalt+Alt+Esc)"
        }
        "another device answered at the known address" => "unter der bekannten Adresse antwortete ein anderes Gerät",
        "Appearance" => "Aussehen",
        "Arrange" => "Anordnen",
        "Arrangement" => "Anordnung",
        "Author" => "Autor",
        "Back" => "Zurück",
        "below this computer" => "unter diesem Rechner",
        "Cancel" => "Abbrechen",
        "cannot save the pairing ({})" => "Kopplung lässt sich nicht speichern ({})",
        "captured" => "erfasst",
        "Check again" => "Erneut prüfen",
        "Clipboard" => "Zwischenablage",
        "Clipboard taken over from {}" => "Zwischenablage von {} übernommen",
        "Code" => "Code",
        "Computers" => "Rechner",
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
            "Kopierter Inhalt bleibt auf diesem Rechner: {} MB sind mehr als erlaubt"
        }
        "Copied password stays on this computer (marked as private)" => {
            "Kopiertes Passwort bleibt auf diesem Rechner (als privat markiert)"
        }
        "Dark" => "Dunkel",
        "Diagnostics copied to the clipboard." => "Diagnose in die Zwischenablage kopiert.",
        "Done" => "Fertig",
        "Done." => "Erledigt.",
        "Drag each computer to the side where it stands on your desk." => {
            "Zieh jeden Rechner auf die Seite, auf der er auf deinem Schreibtisch steht."
        }
        "drop here" => "hier ablegen",
        "Enter a code from another computer" => "Code eines anderen Rechners eingeben",
        "Everything TableCross needs is in place." => "Alles, was TableCross braucht, ist eingerichtet.",
        "Everything was released" => "Alles wurde losgelassen",
        "Files" => "Dateien",
        "Find computers on the network automatically" => "Rechner im Netz automatisch finden",
        "Fingerprint" => "Fingerabdruck",
        "Free software under the GNU General Public License, version 3 or later. It comes without any warranty." => {
            "Freie Software unter der GNU General Public License, Version 3 oder neuer. Ohne jede Gewährleistung."
        }
        "General" => "Allgemein",
        "Go there" => "Dorthin wechseln",
        "Go to the computer on that side" => "Zum Rechner auf dieser Seite wechseln",
        "Install system helper …" => "Systemhelfer installieren …",
        "Keyboard shortcuts" => "Tastenkürzel",
        "Largest content" => "Größter Inhalt",
        "Language" => "Sprache",
        "left of this computer" => "links von diesem Rechner",
        "License" => "Lizenz",
        "Light" => "Hell",
        "Like the system" => "Wie das System",
        "Mouse and keyboard" => "Maus und Tastatur",
        "Mouse and keyboard are not captured right now." => "Maus und Tastatur werden gerade nicht erfasst.",
        "Mouse and keyboard cannot be captured: {}" => "Maus und Tastatur lassen sich nicht erfassen: {}",
        "Mouse and keyboard now control {}" => "Maus und Tastatur steuern jetzt {}",
        "Mouse and keyboard work on this computer" => "Maus und Tastatur arbeiten an diesem Rechner",
        "Name" => "Name",
        "Name of this computer" => "Name dieses Rechners",
        "Network" => "Netzwerk",
        "network error ({})" => "Netzwerkfehler ({})",
        "Next" => "Weiter",
        "no answer for 2 seconds" => "2 Sekunden keine Antwort",
        "No computer on the network shows a code right now." => "Gerade zeigt kein Rechner im Netz einen Code.",
        "none" => "keiner",
        "None yet. Pair a computer on the page “Computers”." => {
            "Noch keiner. Kopple einen Rechner auf der Seite „Rechner“."
        }
        "not available: {}" => "nicht verfügbar: {}",
        "Not connected (searching …)" => "Nicht verbunden (suche …)",
        "Not connected: {}" => "Nicht verbunden: {}",
        "Not found? Enter the address" => "Nicht gefunden? Adresse eingeben",
        "not paired with this computer" => "nicht mit diesem Rechner gekoppelt",
        "not placed" => "nicht platziert",
        "Not placed:" => "Nicht platziert:",
        "Whole desk" => "Ganzer Schreibtisch",
        "Test build {} ({})" => "Testversion {} ({})",
        "Update system helper …" => "Systemhelfer aktualisieren …",
        "Drag the computers to where they stand on your desk. Computers side by side share that edge." => {
            "Zieh die Rechner dorthin, wo sie auf deinem Schreibtisch stehen. Nebeneinanderliegende Rechner teilen sich diese Kante."
        }
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
            "Dieser Rechner hat Version {}. Installiere überall dieselbe Version."
        }
        "The arrangement is shared with all paired computers, so every keyboard reaches every screen." => {
            "Die Anordnung gilt für alle gekoppelten Rechner, so erreicht jede Tastatur jeden Bildschirm."
        }
        "not paired here" => "nicht gekoppelt",
        "To control {} with this keyboard, pair it with this computer as well." => {
            "Um {} mit dieser Tastatur zu steuern, kopple ihn auch mit diesem Rechner."
        }
        "Nothing yet." => "Noch nichts.",
        "now" => "jetzt",
        "off" => "aus",
        "Open Accessibility settings" => "Einstellungen für Bedienungshilfen öffnen",
        "Open Input Monitoring settings" => "Einstellungen für Eingabeüberwachung öffnen",
        "Open settings folder" => "Einstellungsordner öffnen",
        "Pair" => "Koppeln",
        "Pair a computer" => "Rechner koppeln",
        "Paired computers" => "Gekoppelte Rechner",
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
            "Alles loslassen: alle Tasten los, Maus und Tastatur zurück an diesen Rechner"
        }
        "Remove" => "Entfernen",
        "Remove pairing …" => "Kopplung entfernen …",
        "replaced by a newer connection" => "durch eine neuere Verbindung ersetzt",
        "Restart now" => "Jetzt neu starten",
        "right of this computer" => "rechts von diesem Rechner",
        "Screens" => "Bildschirme",
        "Search" => "Suche",
        "Settings" => "Einstellungen",
        "Setup …" => "Einrichtung …",
        "Shares one mouse, keyboard and clipboard between the computers on your desk, on the local network only." => {
            "Teilt eine Maus, eine Tastatur und die Zwischenablage zwischen den Rechnern auf deinem Schreibtisch, nur im lokalen Netz."
        }
        "Show a code here" => "Hier einen Code zeigen",
        "Show log file" => "Logdatei zeigen",
        "Show pairing code" => "Kopplungscode zeigen",
        "Share the clipboard with the other computers" => "Zwischenablage mit den anderen Rechnern teilen",
        "Show TableCross" => "TableCross zeigen",
        "Skip setup" => "Einrichtung überspringen",
        "Source code" => "Quellcode",
        "Start" => "Start",
        "Start TableCross on the other computer as well. Show a code on one of them and type it on the other." => {
            "Starte TableCross auch auf dem anderen Rechner. Lass dir auf einem einen Code zeigen und tippe ihn auf dem anderen ein."
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
            "TableCross spricht nur mit den Rechnern, die du gekoppelt hast, verschlüsselt und nur im lokalen Netz. Kein Konto, keine Cloud, keine Telemetrie. Wenn du es erlaubst, fragt es bei GitHub nach neuen Versionen."
        }
        "Takes effect after a restart of the connection." => "Wirkt nach einem Neustart der Verbindung.",
        "That did not work: {}" => "Das hat nicht geklappt: {}",
        "the other computer cannot be reached" => "der andere Rechner ist nicht erreichbar",
        "the other computer is not showing a code (expired or too many attempts)" => {
            "der andere Rechner zeigt keinen Code (abgelaufen oder zu viele Versuche)"
        }
        "the other computer quit TableCross" => "der andere Rechner hat TableCross beendet",
        "the other computer uses version {} of the protocol, this one {}: please update the older one" => {
            "der andere Rechner nutzt Version {} des Protokolls, dieser {}: bitte den älteren aktualisieren"
        }
        "the other computer was not found on the network" => "der andere Rechner wurde im Netz nicht gefunden",
        "the other computer went to sleep" => "der andere Rechner ist in den Ruhezustand gegangen",
        "Text, formatted text and images. Passwords copied from a password manager stay on this computer." => {
            "Text, formatierter Text und Bilder. Aus einem Passwortmanager kopierte Passwörter bleiben auf diesem Rechner."
        }
        "The pairing code is no longer valid after too many attempts" => {
            "Der Kopplungscode gilt nach zu vielen Versuchen nicht mehr"
        }
        "The system helper could not take {} keyboard(s); their keys also reach this Mac." => {
            "Der Systemhelfer konnte {} Tastatur(en) nicht übernehmen; ihre Tasten kommen auch an diesem Mac an."
        }
        "The system helper is not installed. Without it, typing on other computers stops while a password field is active on this Mac." => {
            "Der Systemhelfer ist nicht installiert. Ohne ihn kommen Tastatureingaben auf anderen Rechnern nicht an, solange an diesem Mac ein Passwortfeld aktiv ist."
        }
        "The system helper is older than the app; please update it." => {
            "Der Systemhelfer ist älter als die App; bitte aktualisieren."
        }
        "The system helper still needs the Input Monitoring permission." => {
            "Der Systemhelfer braucht noch die Berechtigung „Eingabeüberwachung“."
        }
        "Then type it on the other computer." => "Dann am anderen Rechner eintippen.",
        "This computer" => "Dieser Rechner",
        "this computer was asleep" => "dieser Rechner war im Ruhezustand",
        "UDP port" => "UDP-Port",
        "unknown" => "unbekannt",
        "valid for {} min" => "gültig für {} min",
        "Version {} ({})" => "Version {} ({})",
        "While controlling this computer from here, ⌘ acts as Ctrl and the other way round" => {
            "Solange dieser Rechner von hier aus gesteuert wird, wirkt ⌘ wie Strg und umgekehrt"
        }
        "Windows switched off the input capture {} times; it was restarted." => {
            "Windows hat die Eingabeerfassung {}-mal abgeschaltet; sie wurde neu gestartet."
        }
        "Woke from sleep, connecting again" => "Aus dem Ruhezustand aufgewacht, verbinde neu",
        "wrong code" => "falscher Code",
        "{} controls this computer" => "{} steuert diesen Rechner",
        "{} h ago" => "vor {} h",
        "{} min ago" => "vor {} min",
        "{} removed the pairing" => "{} hat die Kopplung entfernt",
        "{} s ago" => "vor {} s",
        "{} was refused: {}" => "{} wurde abgewiesen: {}",
        "Copied on another computer: {}. They come over when the pointer arrives here." => {
            "Auf einem anderen Rechner kopiert: {}. Die Dateien kommen, sobald der Zeiger hierher wechselt."
        }
        "from {}" => "von {}",
        "to {}" => "an {}",
        "{} file(s), done" => "{} Datei(en), fertig",
        "Open folder" => "Ordner öffnen",
        "cancelled" => "abgebrochen",
        "failed: {}" => "fehlgeschlagen: {}",
        "{} files from {} are in {} and on the clipboard" => "{} Dateien von {} liegen in {} und in der Zwischenablage",
        "File transfer with {} failed: {}" => "Dateiübertragung mit {} fehlgeschlagen: {}",
        "A new TableCross is available ({})." => "Eine neue Version von TableCross ist da ({}).",
        "Download" => "Herunterladen",
        "New versions" => "Neue Versionen",
        "May TableCross look for new versions on GitHub now and then? Only the question for the newest release is sent, nothing about you or your computers." => {
            "Darf TableCross ab und zu bei GitHub nach neuen Versionen schauen? Gesendet wird nur die Frage nach der neuesten Version, nichts über dich oder deine Rechner."
        }
        "Yes, look for new versions" => "Ja, nach neuen Versionen schauen",
        "No" => "Nein",
        "Updates" => "Updates",
        "Look for new versions on GitHub" => "Bei GitHub nach neuen Versionen schauen",
        "About TableCross" => "Über TableCross",
        "Arrange the computers on your desk and pair new ones." => {
            "Ordne die Rechner auf deinem Schreibtisch an und kopple neue."
        }
        "How TableCross behaves on this computer." => "Wie sich TableCross auf diesem Rechner verhält.",
        "ID {}" => "ID {}",
        "Setup" => "Einrichtung",
        "This computer, its connections and what happened recently." => {
            "Dieser Rechner, seine Verbindungen und was zuletzt passiert ist."
        }
        "Three short steps, then TableCross is ready." => "Drei kurze Schritte, dann ist TableCross bereit.",
        "Working on this computer" => "Arbeitet an diesem Rechner",
        "connected" => "verbunden",
        "mouse and keyboard captured" => "Maus und Tastatur erfasst",
        "mouse and keyboard not available" => "Maus und Tastatur nicht verfügbar",
        "mouse and keyboard off" => "Maus und Tastatur aus",
        "not connected" => "nicht verbunden",
        "this computer" => "dieser Rechner",
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
