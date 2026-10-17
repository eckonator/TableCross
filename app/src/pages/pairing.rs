//! Pairing: new computers with a code or an address, and the computers paired with this one.

use std::net::SocketAddr;
use std::time::Instant;

use eframe::egui::{self, RichText};
use tc_net::PAIRING_WINDOW;
use tc_session::{Command, Notice, PairTarget, Status};

use super::{card, ok_color, warn_color};
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::texts;
use crate::theme;

impl App {
    pub fn pairing_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(ui, tr("Pairing"), tr("Pair new computers and manage the paired ones."));
        let status = self.status.clone();
        card(ui, tr("Pair a computer"), |ui| self.pairing(ui, &status));
        ui.add_space(14.0);
        card(ui, tr("Paired computers"), |ui| {
            if status.peers.is_empty() {
                ui.label(RichText::new(tr("No computer is paired yet.")).color(theme::colors(ui).muted));
            }
            self.peer_options(ui, &status);
        });
    }

    pub fn pairing(&mut self, ui: &mut egui::Ui, status: &Status) {
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            let p = theme::colors(ui);
            ui.label(RichText::new(tr("Show a code here")).font(egui::FontId::new(14.0, theme::semibold())));
            ui.label(RichText::new(tr("Then type it on the other computer.")).color(p.muted));
            ui.add_space(10.0);
            match &status.pairing {
                Some((code, until)) => {
                    egui::Frame::new()
                        .fill(p.accent_soft)
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin::symmetric(22, 12))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(code.to_string())
                                    .font(egui::FontId::new(38.0, theme::semibold()))
                                    .color(p.accent),
                            );
                        });
                    let left = until.saturating_duration_since(Instant::now()).as_secs();
                    let remaining = format!("{}:{:02}", left / 60, left % 60);
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(trf("valid for {} min", &[&remaining]))
                            .small()
                            .color(p.muted),
                    );
                    if ui.horizontal(|ui| ui.button(tr("Stop"))).inner.clicked() {
                        self.send(Command::StopPairing);
                    }
                }
                None => {
                    if ui
                        .horizontal(|ui| theme::primary_button(ui, tr("Show pairing code")))
                        .inner
                        .clicked()
                    {
                        self.send(Command::StartPairing);
                        if let Some(demo) = &mut self.demo {
                            demo.pairing =
                                tc_net::PairingCode::parse("482913").map(|c| (c, Instant::now() + PAIRING_WINDOW));
                        }
                    }
                }
            }

            let ui = &mut cols[1];
            let p = theme::colors(ui);
            ui.label(
                RichText::new(tr("Enter a code from another computer"))
                    .font(egui::FontId::new(14.0, theme::semibold())),
            );
            ui.add_space(4.0);
            let showing: Vec<_> = status.discovered.iter().filter(|d| d.pairing).collect();
            if showing.is_empty() {
                ui.label(RichText::new(tr("No computer on the network shows a code right now.")).color(p.muted));
            } else if self
                .pair_target
                .is_none_or(|t| !showing.iter().any(|d| d.fingerprint == t))
            {
                self.pair_target = showing.first().map(|d| d.fingerprint);
            }
            for d in &showing {
                ui.radio_value(&mut self.pair_target, Some(d.fingerprint), &d.name);
            }
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.code_input)
                    .desired_width(150.0)
                    .font(egui::FontId::new(22.0, theme::semibold()))
                    .hint_text("123 456")
                    .margin(egui::Margin::symmetric(10, 6)),
            );
            ui.collapsing(tr("Not found? Enter the address"), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.address_input)
                        .desired_width(180.0)
                        .hint_text("192.168.1.20:24800")
                        .margin(egui::Margin::symmetric(8, 6)),
                );
            });
            let code_ok = tc_net::PairingCode::parse(&self.code_input).is_some();
            let address: Option<SocketAddr> = self.address_input.trim().parse().ok();
            let target = address
                .map(PairTarget::Address)
                .or(self.pair_target.map(PairTarget::Discovered));
            ui.horizontal(|ui| {
                let enabled = code_ok && target.is_some() && !status.pairing_busy;
                let pair = ui
                    .add_enabled_ui(enabled, |ui| theme::primary_button(ui, tr("Pair")))
                    .inner;
                if pair.clicked()
                    && let Some(target) = target
                {
                    self.send(Command::Pair {
                        code: self.code_input.clone(),
                        target,
                    });
                    self.code_input.clear();
                }
                if status.pairing_busy {
                    ui.spinner();
                }
            });
            // Result of the last attempt
            if let Some((at, notice)) = status
                .notices
                .iter()
                .rev()
                .find(|(_, n)| matches!(n, Notice::Paired(_) | Notice::PairError(_)))
                && at.elapsed().is_ok_and(|d| d.as_secs() < 30)
            {
                let color = if matches!(notice, Notice::Paired(_)) {
                    ok_color(ui)
                } else {
                    warn_color(ui)
                };
                ui.colored_label(color, texts::notice(notice));
            }
        });
    }

    fn peer_options(&mut self, ui: &mut egui::Ui, status: &Status) {
        let p = theme::colors(ui);
        for (i, peer) in status.peers.iter().enumerate() {
            if i > 0 {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&peer.name).strong());
                if peer.link.is_some() {
                    theme::chip(ui, tr("connected"), p.ok, p.ok_soft);
                } else {
                    theme::chip(ui, tr("not connected"), p.muted, p.field);
                }
                theme::chip(ui, &trf("ID {}", &[&peer.fingerprint.short()]), p.muted, p.field);
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let mut swap = peer.swap_cmd_ctrl;
                if ui
                    .checkbox(&mut swap, tr("Swap ⌘ and Ctrl"))
                    .on_hover_text(tr(
                        "While controlling this computer from here, ⌘ acts as Ctrl and the other way round",
                    ))
                    .changed()
                {
                    self.send(Command::SwapCmdCtrl {
                        peer: peer.fingerprint,
                        swap,
                    });
                }
                if peer.link.is_some() && ui.button(tr("Go there")).clicked() {
                    self.send(Command::SwitchTo(Some(peer.fingerprint)));
                }
                if self.confirm_unpair == Some(peer.fingerprint) {
                    ui.label(tr("Really remove?"));
                    if ui.button(RichText::new(tr("Remove")).color(warn_color(ui))).clicked() {
                        self.send(Command::Unpair(peer.fingerprint));
                        self.confirm_unpair = None;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        self.confirm_unpair = None;
                    }
                } else if ui.button(tr("Remove pairing …")).clicked() {
                    self.confirm_unpair = Some(peer.fingerprint);
                }
            });
        }
    }
}
