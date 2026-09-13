//! Setup in three steps: permissions, pairing, arrangement.

use eframe::egui;
use tc_input::Problem;

use super::{card, ok_color};
use crate::app::App;
use crate::i18n::{tr, trf};

impl App {
    pub fn setup(&mut self, ui: &mut egui::Ui, step: usize) {
        let status = self.status.clone();
        let titles = [tr("Permissions"), tr("Pair"), tr("Arrange")];
        self.page_header(ui, tr("Setup"), tr("Three short steps, then TableCross is ready."));
        stepper(ui, &titles, step);
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
            if crate::theme::primary_button(ui, next).clicked() {
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

/// Numbered circles joined by a line; the current step is highlighted.
fn stepper(ui: &mut egui::Ui, titles: &[&str], current: usize) {
    let p = crate::theme::colors(ui);
    let width = ui.available_width().min(620.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 58.0), egui::Sense::hover());
    let step_w = width / titles.len() as f32;
    let y = rect.min.y + 16.0;
    let center = |i: usize| egui::pos2(rect.min.x + step_w * (i as f32 + 0.5), y);
    for i in 0..titles.len() - 1 {
        let color = if i < current { p.accent } else { p.border };
        ui.painter().line_segment(
            [center(i) + egui::vec2(16.0, 0.0), center(i + 1) - egui::vec2(16.0, 0.0)],
            egui::Stroke::new(2.0, color),
        );
    }
    for (i, title) in titles.iter().enumerate() {
        let c = center(i);
        let done = i < current;
        let active = i == current;
        let (fill, fg) = if active || done {
            (p.accent, p.on_accent)
        } else {
            (p.field, p.muted)
        };
        ui.painter().circle_filled(c, 14.0, fill);
        if !active && !done {
            ui.painter().circle_stroke(c, 14.0, egui::Stroke::new(1.0, p.border));
        }
        let label = if done { "✔".to_string() } else { (i + 1).to_string() };
        ui.painter().text(
            c,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(13.0, crate::theme::semibold()),
            fg,
        );
        let font = if active {
            egui::FontId::new(13.0, crate::theme::semibold())
        } else {
            egui::FontId::proportional(13.0)
        };
        ui.painter().text(
            c + egui::vec2(0.0, 22.0),
            egui::Align2::CENTER_TOP,
            title,
            font,
            if active { p.text } else { p.muted },
        );
    }
    ui.add_space(8.0);
}
