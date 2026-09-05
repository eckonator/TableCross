//! About: version, license, author.

use eframe::egui::{self, RichText};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};

impl App {
    pub fn about_page(&mut self, ui: &mut egui::Ui) {
        card(ui, "TableCross", |ui| {
            ui.label(trf("Version {}", &[&env!("CARGO_PKG_VERSION")]));
            ui.add_space(4.0);
            ui.label(tr(
                "Shares one mouse, keyboard and clipboard between the computers on your desk, \
                 on the local network only.",
            ));
            ui.add_space(8.0);
            ui.label(RichText::new(tr("License")).strong());
            ui.label(tr(
                "Free software under the GNU General Public License, version 3 or later. \
                 It comes without any warranty.",
            ));
            ui.add_space(8.0);
            ui.label(RichText::new(tr("Author")).strong());
            ui.label("Markus Eckert");
            ui.add_space(8.0);
            ui.hyperlink_to(tr("Source code"), env!("CARGO_PKG_REPOSITORY"));
            ui.add_space(8.0);
            ui.label(RichText::new(tr("Privacy")).strong());
            ui.label(tr(
                "TableCross talks only to the computers you paired, encrypted, on your local network. \
                 No account, no cloud, no telemetry.",
            ));
        });
    }
}
