//! Status: this computer, the paired computers, problems with a way out, recent messages.

use eframe::egui::{self, RichText};
use tc_input::Problem;
use tc_session::{Command, InputState, Status};

use super::{card, ok_color, warn_color};
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::texts;

impl App {
    pub fn status_page(&mut self, ui: &mut egui::Ui) {
        let status = self.status.clone();
        self.problems(ui, &status);
        ui.add_space(8.0);
        card(ui, tr("This computer"), |ui| {
            egui::Grid::new("me")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label(tr("Name"));
                    ui.label(RichText::new(&status.name).strong());
                    ui.end_row();
                    ui.label(tr("Fingerprint"));
                    ui.label(RichText::new(status.fingerprint.map(|f| f.short()).unwrap_or_default()).monospace());
                    ui.end_row();
                    ui.label(tr("Port"));
                    ui.label(format!("UDP {}", status.port));
                    ui.end_row();
                    ui.label(tr("Screens"));
                    let screens: Vec<String> = status
                        .screens
                        .iter()
                        .map(|s| format!("{}×{}", s.width, s.height))
                        .collect();
                    ui.label(if screens.is_empty() {
                        tr("unknown").to_string()
                    } else {
                        screens.join(", ")
                    });
                    ui.end_row();
                    ui.label(tr("Mouse and keyboard"));
                    match &status.input {
                        InputState::Running => ui.colored_label(ok_color(ui), tr("captured")),
                        InputState::Off => ui.label(tr("off")),
                        InputState::Failed(e) => ui.colored_label(warn_color(ui), trf("not available: {}", &[e])),
                    };
                    ui.end_row();
                });
        });
        ui.add_space(8.0);
        card(ui, tr("Paired computers"), |ui| {
            if status.peers.is_empty() {
                ui.label(tr("None yet. Pair a computer on the page “Computers”."));
            }
            for peer in &status.peers {
                ui.horizontal(|ui| {
                    let connected = peer.link.is_some();
                    let color = if connected {
                        ok_color(ui)
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    super::dot(ui, color, connected);
                    ui.label(RichText::new(&peer.name).strong());
                    ui.label(RichText::new(texts::side(peer.side)).weak());
                });
                ui.indent(peer.fingerprint.short(), |ui| match &peer.link {
                    Some(link) => {
                        let rtt = link
                            .rtt
                            .map_or_else(|| "–".to_string(), |r| format!("{:.1} ms", r.as_secs_f64() * 1000.0));
                        ui.label(trf(
                            "Connected at {} · round trip {} · lost {} of {} · reconnects {}",
                            &[
                                &link.address,
                                &rtt,
                                &link.probes_lost,
                                &link.probes_sent,
                                &peer.reconnects,
                            ],
                        ));
                    }
                    None => {
                        let text = match &peer.refusal {
                            Some(r) => trf("Not connected: {}", &[&texts::refusal(r)]),
                            None => tr("Not connected (searching …)").to_string(),
                        };
                        ui.label(RichText::new(text).weak());
                    }
                });
                ui.add_space(4.0);
            }
        });
        ui.add_space(8.0);
        card(ui, tr("Recent messages"), |ui| {
            if status.notices.is_empty() {
                ui.label(RichText::new(tr("Nothing yet.")).weak());
            }
            egui::Grid::new("notices")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for (at, notice) in status.notices.iter().rev().take(12) {
                        ui.label(RichText::new(texts::ago(*at)).weak());
                        ui.label(texts::notice(notice));
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Copy diagnostics")).clicked() {
                    ui.ctx().copy_text(self.diagnostics(&status));
                    self.show_info(tr("Diagnostics copied to the clipboard."));
                }
                if let Some(log) = self.log_file.clone()
                    && ui.button(tr("Show log file")).clicked()
                {
                    super::open(&log.display().to_string());
                }
            });
        });
    }

    /// Problems at the top, each with what can be done.
    fn problems(&mut self, ui: &mut egui::Ui, status: &Status) {
        for problem in &status.problems {
            egui::Frame::group(ui.style())
                .inner_margin(10.0)
                .corner_radius(8.0)
                .stroke(egui::Stroke::new(1.5, warn_color(ui)))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(texts::problem(problem)).color(warn_color(ui)));
                    ui.horizontal(|ui| self.problem_actions(ui, problem));
                });
            ui.add_space(4.0);
        }
    }

    pub fn problem_actions(&mut self, ui: &mut egui::Ui, problem: &Problem) {
        match problem {
            #[cfg(target_os = "macos")]
            Problem::AccessibilityMissing => {
                if ui.button(tr("Open Accessibility settings")).clicked() {
                    super::open(super::ACCESSIBILITY_SETTINGS);
                }
                if ui.button(tr("Check again")).clicked() {
                    self.send(Command::RetryInput);
                }
            }
            #[cfg(target_os = "macos")]
            Problem::HelperMissing | Problem::HelperOutdated | Problem::SecureInput => {
                let busy = self.job.is_some();
                if ui
                    .add_enabled(!busy, egui::Button::new(tr("Install system helper …")))
                    .clicked()
                {
                    self.run_job(|| tc_session::helper_install::install().map_err(|e| e.to_string()));
                }
            }
            #[cfg(target_os = "macos")]
            Problem::HelperNoAccess => {
                if ui.button(tr("Open Input Monitoring settings")).clicked() {
                    super::open(tc_session::helper_install::INPUT_MONITORING_SETTINGS);
                }
            }
            Problem::CaptureStopped if ui.button(tr("Check again")).clicked() => {
                self.send(Command::RetryInput);
            }
            _ => {}
        }
    }

    /// Plain text for bug reports (English, so anyone can read it).
    fn diagnostics(&self, status: &Status) -> String {
        let mut out = format!(
            "TableCross {} on {} {}\nname: {}\nfingerprint: {}\nport: {}\ninput: {:?}\nscreens: {:?}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            status.name,
            status.fingerprint.map(|f| f.to_string()).unwrap_or_default(),
            status.port,
            status.input,
            status.screens,
        );
        out.push_str(&format!(
            "controlling: {:?}\ncontrolled by: {:?}\n",
            status.controlling, status.controlled_by
        ));
        for p in &status.problems {
            out.push_str(&format!("problem: {p}\n"));
        }
        for peer in &status.peers {
            out.push_str(&format!(
                "peer {} ({}): side {:?}, swap {}, reconnects {}, refusal {:?}\n",
                peer.name,
                peer.fingerprint.short(),
                peer.side,
                peer.swap_cmd_ctrl,
                peer.reconnects,
                peer.refusal
            ));
            if let Some(link) = &peer.link {
                out.push_str(&format!(
                    "  link {} dialed {} rtt {:?} quic {:?} lost {}/{} screens {:?}\n",
                    link.address,
                    link.dialed,
                    link.rtt,
                    link.quic_rtt,
                    link.probes_lost,
                    link.probes_sent,
                    link.screens
                ));
            }
        }
        for d in &status.discovered {
            out.push_str(&format!(
                "found {} ({}) at {:?}, version {}\n",
                d.name,
                d.fingerprint.short(),
                d.addresses,
                d.version
            ));
        }
        for (at, notice) in status.notices.iter().rev().take(50) {
            let secs = at.elapsed().map_or(0, |d| d.as_secs());
            out.push_str(&format!("-{secs}s {notice}\n"));
        }
        out
    }
}
