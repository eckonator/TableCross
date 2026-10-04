//! About: version, license, author.

use eframe::egui::{self, RichText};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};

impl App {
    pub fn about_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(ui, tr("About TableCross"), "");
        card(ui, "TableCross", |ui| {
            let commit: String = env!("TC_GIT_HASH").chars().take(7).collect();
            let version = if crate::update::is_release() {
                trf("Version {} ({})", &[&env!("CARGO_PKG_VERSION"), &commit])
            } else {
                trf("Test build {} ({})", &[&env!("CARGO_PKG_VERSION"), &commit])
            };
            ui.horizontal(|ui| {
                ui.label(version);
                self.check_button(ui);
            });
            ui.add_space(4.0);
            ui.label(tr(
                "Shares one mouse, keyboard and clipboard between the computers on your desk, \
                 on the local network only.",
            ));
            ui.add_space(14.0);
            ui.label(RichText::new(tr("License")).font(egui::FontId::new(14.0, crate::theme::semibold())));
            ui.label(tr(
                "Free software under the GNU General Public License, version 3 or later. \
                 It comes without any warranty.",
            ));
            ui.add_space(14.0);
            ui.label(RichText::new(tr("Author")).font(egui::FontId::new(14.0, crate::theme::semibold())));
            ui.label("Markus Eckert");
            ui.add_space(14.0);
            super::link(ui, tr("Source code"), env!("CARGO_PKG_REPOSITORY"));
            ui.add_space(14.0);
            ui.label(RichText::new(tr("Privacy")).font(egui::FontId::new(14.0, crate::theme::semibold())));
            ui.label(tr(
                "TableCross talks only to the computers you paired, encrypted, on your local network. \
                 No account, no cloud, no telemetry. If you allow it, it asks GitHub for new versions.",
            ));
        });
    }
}
