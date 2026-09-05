//! Setup in three steps: permissions, pairing, arrangement.

use eframe::egui::{self, RichText};
use tc_input::Problem;

use super::{card, ok_color};
use crate::app::App;
use crate::i18n::{tr, trf};

impl App {
    pub fn setup(&mut self, ui: &mut egui::Ui, step: usize) {
        let status = self.status.clone();
        let titles = [tr("Permissions"), tr("Pair"), tr("Arrange")];
        ui.horizontal(|ui| {
            for (i, title) in titles.iter().enumerate() {
                let text = format!("{}  {}", i + 1, title);
                let text = if i == step {
                    RichText::new(text).strong().size(16.0)
                } else {
                    RichText::new(text).weak()
                };
                ui.label(text);
                if i + 1 < titles.len() {
                    ui.label(RichText::new("›").weak());
                }
            }
        });
        ui.add_space(10.0);
        match step {
            0 => card(ui, tr("Permissions"), |ui| {
                let problems: Vec<Problem> = status
                    .problems
                    .iter()
                    .filter(|p| !matches!(p, Problem::HooksRemoved(_)))
                    .cloned()
                    .collect();
                if problems.is_empty() {
                    ui.colored_label(ok_color(ui), tr("Everything TableCross needs is in place."));
                } else {
                    ui.label(tr(
                        "TableCross needs a few things before it can share mouse and keyboard:",
                    ));
                    ui.add_space(6.0);
                    for problem in &problems {
                        ui.label(format!("• {}", crate::texts::problem(problem)));
                        ui.horizontal(|ui| self.problem_actions(ui, problem));
                        ui.add_space(4.0);
                    }
                }
            }),
            1 => card(ui, tr("Pair"), |ui| {
                ui.label(tr(
                    "Start TableCross on the other computer as well. Show a code on one of them and type it on the other.",
                ));
                ui.add_space(6.0);
                self.pairing(ui, &status);
                if !status.peers.is_empty() {
                    ui.add_space(6.0);
                    let names: Vec<_> = status.peers.iter().map(|p| p.name.as_str()).collect();
                    ui.colored_label(ok_color(ui), trf("Paired: {}", &[&names.join(", ")]));
                }
            }),
            _ => card(ui, tr("Arrange"), |ui| self.arrangement(ui, &status)),
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if step > 0 && ui.button(tr("Back")).clicked() {
                self.setup_step = Some(step - 1);
            }
            let last = step + 1 == titles.len();
            let next = if last { tr("Done") } else { tr("Next") };
            if ui.button(RichText::new(next).strong()).clicked() {
                if last {
                    self.finish_setup();
                } else {
                    self.setup_step = Some(step + 1);
                }
            }
            if !last && ui.button(tr("Skip setup")).clicked() {
                self.finish_setup();
            }
        });
    }

    fn finish_setup(&mut self) {
        self.setup_step = None;
        self.settings.setup_done = true;
        self.save_settings();
        self.page = crate::app::Page::Status;
    }
}
