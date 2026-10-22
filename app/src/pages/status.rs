//! Status: this computer, the paired computers, problems with a way out, recent messages.

use eframe::egui::{self, RichText};
use tc_input::Problem;
use tc_session::{Command, InputState, Status, TransferState};

use super::{card, ok_color};
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::texts;
use crate::theme;

impl App {
    pub fn status_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            tr("Status"),
            tr("This computer, its connections and what happened recently."),
        );
        let status = self.status.clone();
        self.update_notes(ui);
        self.icons_note(ui);
        self.problems(ui, &status);
        ui.add_space(14.0);
        card(ui, "", |ui| self.this_computer(ui, &status));
        ui.add_space(14.0);
        card(ui, tr("Paired computers"), |ui| {
            if status.peers.is_empty() {
                ui.label(RichText::new(tr("None yet. Pair a computer on the page “Computers”.")).weak());
            }
            for (i, peer) in status.peers.iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }
                peer_row(ui, peer);
            }
        });
        if !status.transfers.is_empty() || status.files_waiting.is_some() {
            ui.add_space(14.0);
            card(ui, tr("Files"), |ui| self.transfers(ui, &status));
        }
        ui.add_space(14.0);
        card(ui, tr("Recent messages"), |ui| {
            if status.notices.is_empty() {
                ui.label(RichText::new(tr("Nothing yet.")).weak());
            }
            let p = theme::colors(ui);
            for (at, notice) in status.notices.iter().rev().take(12) {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.0, notice_color(&p, notice));
                    ui.add_sized(
                        [86.0, 18.0],
                        egui::Label::new(RichText::new(texts::ago(*at)).small().color(p.muted)),
                    );
                    ui.label(texts::notice(notice));
                });
            }
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

    fn transfers(&mut self, ui: &mut egui::Ui, status: &Status) {
        if let Some(waiting) = &status.files_waiting {
            if waiting.pasteable {
                let text = trf(
                    "Copied on {}: {}. Paste them where you want them, with ⌘V in the Finder.",
                    &[&waiting.peer, &waiting.label],
                );
                theme::note(ui, theme::Tone::Info, &text);
            } else {
                let text = trf("Copied on {}: {}.", &[&waiting.peer, &waiting.label]);
                theme::note(ui, theme::Tone::Info, &text);
                ui.add_space(4.0);
                if ui.button(tr("Save files …")).clicked() {
                    self.save_files();
                }
            }
            ui.add_space(8.0);
        }
        for t in status.transfers.iter().rev() {
            let direction = if t.incoming {
                trf("from {}", &[&t.peer])
            } else {
                trf("to {}", &[&t.peer])
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&t.label).strong());
                ui.label(RichText::new(direction).weak());
            });
            let fraction = if t.total == 0 {
                1.0
            } else {
                t.done as f32 / t.total as f32
            };
            let size = format!("{} / {}", texts::bytes(t.done), texts::bytes(t.total));
            match &t.state {
                TransferState::Running => {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(280.0)
                                .fill(theme::colors(ui).accent)
                                .text(RichText::new(size).color(theme::colors(ui).on_accent)),
                        );
                        if ui.button(tr("Cancel")).clicked() {
                            self.send(Command::CancelTransfer(t.id));
                        }
                    });
                }
                TransferState::Done(folder) => {
                    ui.horizontal(|ui| {
                        ui.colored_label(ok_color(ui), trf("{} file(s), done", &[&t.count]));
                        if let Some(folder) = folder
                            && ui.small_button(tr("Open folder")).clicked()
                        {
                            super::open(&folder.display().to_string());
                        }
                    });
                }
                TransferState::Cancelled => {
                    ui.label(RichText::new(tr("cancelled")).weak());
                }
                TransferState::Failed(e) => {
                    ui.colored_label(theme::colors(ui).bad, trf("failed: {}", &[e]));
                }
            }
            ui.add_space(4.0);
        }
        ui.add_space(4.0);
        if ui.button(tr("Clear list")).clicked() {
            self.send(Command::ClearFiles);
        }
    }

    /// The question whether to look for new versions, or the note that there is one.
    /// GNOME shows no status icons without an extension: no menu, and a closed window is hard to find.
    fn icons_note(&mut self, ui: &mut egui::Ui) {
        if !self.icons_missing {
            return;
        }
        const INSTALL: &str = "sudo apt install gnome-shell-extension-appindicator";
        let text = tr(
            "This desktop shows no status icons, so the TableCross symbol and its menu are missing. On \
             GNOME, install the AppIndicator extension, log out and in again and switch it on in the app \
             “Extensions” (on Debian and Ubuntu:",
        );
        let mut copied = false;
        theme::banner(ui, theme::Tone::Info, &format!("{text} {INSTALL})."), |ui| {
            copied = ui.button(tr("Copy command")).clicked();
        });
        if copied {
            ui.ctx().copy_text(INSTALL.into());
            self.show_info(tr("Command copied to the clipboard."));
        }
        ui.add_space(10.0);
    }

    fn update_notes(&mut self, ui: &mut egui::Ui) {
        if let Some(new) = self.new_version.clone() {
            use crate::self_update::Progress;
            let text = trf("A new TableCross is available ({}).", &[&new.version]);
            let progress = self.update_progress.clone();
            let mut install = false;
            theme::banner(ui, theme::Tone::Ok, &text, |ui| match &progress {
                Some(Progress::Downloading) => {
                    ui.spinner();
                    ui.label(tr("Downloading …"));
                }
                Some(Progress::Installing | Progress::Restart) => {
                    ui.spinner();
                    ui.label(tr("Installing, TableCross starts again by itself …"));
                }
                _ => {
                    if new.asset.is_some() {
                        install = theme::primary_button(ui, tr("Update now")).clicked();
                    }
                    super::link(ui, tr("Download"), &new.url);
                    if let Some(Progress::Failed(e)) = &progress {
                        ui.label(RichText::new(trf("The update did not work: {}", &[e])).color(theme::colors(ui).bad));
                    }
                }
            });
            if install {
                self.install_update(ui.ctx());
            }
            ui.add_space(10.0);
        }
        if self.settings.update_check.is_none() && self.demo.is_none() {
            let mut answer = None;
            card(ui, tr("New versions"), |ui| {
                ui.label(tr(
                    "May TableCross look for new versions on GitHub now and then? Only the question for the \
                     newest release is sent, nothing about you or your computers.",
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr("Yes, look for new versions")).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(tr("No")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer.is_some() {
                self.settings.update_check = answer;
                self.save_settings();
            }
            ui.add_space(14.0);
        }
    }

    /// Problems at the top, each with what can be done.
    fn problems(&mut self, ui: &mut egui::Ui, status: &Status) {
        for problem in &status.problems {
            theme::banner(ui, theme::Tone::Warn, &texts::problem(problem), |ui| {
                self.problem_actions(ui, problem)
            });
            ui.add_space(10.0);
        }
    }

    /// Name, state and the facts of this computer, next to a little monitor.
    fn this_computer(&self, ui: &mut egui::Ui, status: &Status) {
        let p = theme::colors(ui);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(84.0, 64.0), egui::Sense::hover());
            let inner = theme::monitor(ui.painter(), rect, p.accent, p.accent_soft);
            ui.painter().text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                "✔",
                egui::FontId::proportional(18.0),
                p.accent,
            );
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(tr("This computer")).small().color(p.muted));
                ui.label(RichText::new(&status.name).font(egui::FontId::new(20.0, theme::semibold())));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    match &status.input {
                        InputState::Running => theme::chip(ui, tr("mouse and keyboard captured"), p.ok, p.ok_soft),
                        InputState::Off => theme::chip(ui, tr("mouse and keyboard off"), p.muted, p.field),
                        InputState::Failed(_) => {
                            theme::chip(ui, tr("mouse and keyboard not available"), p.warn, p.warn_soft)
                        }
                    }
                    let screens = status.screens.len();
                    theme::chip(ui, &trf("{} screen(s)", &[&screens]), p.muted, p.field);
                    theme::chip(ui, &format!("UDP {}", status.port), p.muted, p.field);
                    let fingerprint = status.fingerprint.map(|f| f.short()).unwrap_or_default();
                    theme::chip(ui, &trf("ID {}", &[&fingerprint]), p.muted, p.field);
                });
                if let InputState::Failed(e) = &status.input {
                    ui.label(RichText::new(e).small().color(p.warn));
                }
            });
        });
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
            Problem::HelperOutdated => {
                let busy = self.job.is_some();
                if ui
                    .add_enabled(!busy, egui::Button::new(tr("Update system helper …")))
                    .clicked()
                {
                    self.run_job(|| tc_session::helper_install::update().map_err(|e| e.to_string()));
                    self.helper_prompt = Some(std::time::Instant::now() + crate::app::HELPER_PROMPT_FOR);
                }
            }
            #[cfg(target_os = "macos")]
            Problem::HelperMissing | Problem::SecureInput => {
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
                    tc_session::helper_install::show_permission_place();
                }
            }
            #[cfg(target_os = "macos")]
            Problem::LoginAgentNoAccess => {
                if ui.button(tr("Open Accessibility settings")).clicked() {
                    tc_session::helper_install::show_login_permission_place();
                }
            }
            #[cfg(target_os = "macos")]
            Problem::UniversalControl => {
                if ui.button(tr("Switch off Universal Control")).clicked() {
                    match tc_input::universal_control::turn_off() {
                        Ok(()) => tracing::info!("Universal Control switched off"),
                        Err(e) => {
                            tracing::warn!("cannot switch off Universal Control: {e}");
                            self.show_info(trf("Could not switch off Universal Control: {}", &[&e]));
                        }
                    }
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
                    "  link {} app {} dialed {} rtt {:?} quic {:?} lost {}/{} screens {:?}\n",
                    link.address,
                    link.app_version,
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

fn notice_color(p: &theme::Palette, notice: &tc_session::Notice) -> egui::Color32 {
    use tc_session::Notice::*;
    match notice {
        Connected(_) | Paired(_) | FilesReceived(..) | ClipboardReceived(_) => p.ok,
        Disconnected(..) | Refused(..) | PairError(_) | PairingFailed(_) | Problem(_) | InputFailed(_)
        | TransferFailed(..) | ClipboardSkipped(_) | LayoutMissing(..) => p.warn,
        _ => p.accent,
    }
}

/// One paired computer: name, where it stands, connection details.
fn peer_row(ui: &mut egui::Ui, peer: &tc_session::PeerView) {
    let p = theme::colors(ui);
    let connected = peer.link.is_some();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 36.0), egui::Sense::hover());
        let (frame, screen) = if connected {
            (p.accent, p.accent_soft)
        } else {
            (p.border, p.field)
        };
        theme::monitor(ui.painter(), rect, frame, screen);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&peer.name).font(egui::FontId::new(15.0, theme::semibold())));
                if connected {
                    theme::chip(ui, tr("connected"), p.ok, p.ok_soft);
                } else {
                    theme::chip(ui, tr("not connected"), p.muted, p.field);
                }
                theme::chip(ui, texts::side(peer.side), p.muted, p.field);
                if let Some(link) = &peer.link
                    && link.app_version != env!("CARGO_PKG_VERSION")
                {
                    let version = if link.app_version.is_empty() {
                        "?"
                    } else {
                        &link.app_version
                    };
                    let text = trf("version {}", &[&version]);
                    let response = ui.scope(|ui| theme::chip(ui, &text, p.warn, p.warn_soft)).response;
                    response.on_hover_text(trf(
                        "This computer runs version {}. Install the same version everywhere.",
                        &[&env!("CARGO_PKG_VERSION")],
                    ));
                }
            });
            let details = match &peer.link {
                Some(link) => {
                    let rtt = link
                        .rtt
                        .map_or_else(|| "–".to_string(), |r| format!("{:.1} ms", r.as_secs_f64() * 1000.0));
                    trf(
                        "Connected at {} · round trip {} · lost {} of {} · reconnects {}",
                        &[
                            &link.address,
                            &rtt,
                            &link.probes_lost,
                            &link.probes_sent,
                            &peer.reconnects,
                        ],
                    )
                }
                None => match &peer.refusal {
                    Some(r) => trf("Not connected: {}", &[&texts::refusal(r)]),
                    None => tr("Not connected (searching …)").to_string(),
                },
            };
            ui.label(RichText::new(details).small().color(p.muted));
        });
    });
}
