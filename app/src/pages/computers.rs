//! Computers: arranging them around this one by dragging, pairing, per-computer options.

use std::net::SocketAddr;
use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Vec2};
use tc_core::Side;
use tc_net::{Fingerprint, PAIRING_WINDOW};
use tc_session::{Command, Notice, PairTarget, Status};

use super::{card, ok_color, warn_color};
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::texts;

const TILE: Vec2 = Vec2::new(150.0, 86.0);

/// What is dragged in the arrangement.
#[derive(Clone, Copy)]
struct Dragged(Fingerprint);

impl App {
    pub fn computers_page(&mut self, ui: &mut egui::Ui) {
        let status = self.status.clone();
        card(ui, tr("Arrangement"), |ui| self.arrangement(ui, &status));
        ui.add_space(8.0);
        card(ui, tr("Pair a computer"), |ui| self.pairing(ui, &status));
        ui.add_space(8.0);
        if !status.peers.is_empty() {
            card(ui, tr("Paired computers"), |ui| self.peer_options(ui, &status));
        }
    }

    /// This computer in the middle, the four sides around it; computers are dragged onto a side.
    pub fn arrangement(&mut self, ui: &mut egui::Ui, status: &Status) {
        ui.label(tr("Drag each computer to the side where it stands on your desk."));
        ui.add_space(8.0);
        let mut dropped: Option<(Fingerprint, Option<Side>)> = None;
        let me = status.name.clone();
        egui::Grid::new("arrangement").spacing([10.0, 10.0]).show(ui, |ui| {
            for row in [
                [None, Some(Some(Side::Top)), None],
                [Some(Some(Side::Left)), Some(None), Some(Some(Side::Right))],
                [None, Some(Some(Side::Bottom)), None],
            ] {
                for cell in row {
                    match cell {
                        None => {
                            ui.allocate_exact_size(TILE, egui::Sense::hover());
                        }
                        Some(None) => {
                            let (rect, _) = ui.allocate_exact_size(TILE, egui::Sense::hover());
                            let fill = ui.visuals().selection.bg_fill;
                            ui.painter().rect_filled(rect, 8.0, fill);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("{me}\n{}", tr("(this computer)")),
                                egui::FontId::proportional(13.0),
                                ui.visuals().strong_text_color(),
                            );
                        }
                        Some(Some(side)) => {
                            if let Some(fp) = self.side_zone(ui, status, side) {
                                dropped = Some((fp, Some(side)));
                            }
                        }
                    }
                }
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        // Computers without a place
        let frame = egui::Frame::new()
            .inner_margin(8.0)
            .corner_radius(8.0)
            .stroke(egui::Stroke::new(
                1.0,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ));
        let (_, payload) = ui.dnd_drop_zone::<Dragged, _>(frame, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Not placed:")).weak());
                let unplaced: Vec<_> = status.peers.iter().filter(|p| p.side.is_none()).collect();
                if unplaced.is_empty() {
                    ui.label(RichText::new(tr("none")).weak());
                }
                for peer in unplaced {
                    chip(ui, peer.fingerprint, &peer.name, peer.link.is_some());
                }
            });
        });
        if let Some(payload) = payload {
            dropped = Some((payload.0, None));
        }
        if let Some((peer, side)) = dropped {
            self.place(peer, side);
        }
    }

    fn place(&mut self, peer: Fingerprint, side: Option<Side>) {
        if let Some(demo) = &mut self.demo {
            for p in &mut demo.peers {
                if p.side == side && side.is_some() {
                    p.side = None;
                }
                if p.fingerprint == peer {
                    p.side = side;
                }
            }
        }
        self.send(Command::Place { peer, side });
    }

    /// One side around this computer; returns a computer dropped onto it.
    fn side_zone(&mut self, ui: &mut egui::Ui, status: &Status, side: Side) -> Option<Fingerprint> {
        let occupant = status.peers.iter().find(|p| p.side == Some(side));
        let frame = egui::Frame::new().corner_radius(8.0).stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ));
        let (_, payload) = ui.dnd_drop_zone::<Dragged, _>(frame, |ui| {
            ui.set_min_size(TILE - Vec2::splat(2.0));
            ui.set_max_size(TILE - Vec2::splat(2.0));
            ui.centered_and_justified(|ui| match occupant {
                Some(peer) => chip(ui, peer.fingerprint, &peer.name, peer.link.is_some()),
                None => {
                    ui.label(RichText::new(tr("drop here")).weak());
                }
            });
        });
        payload.map(|p| p.0)
    }

    pub fn pairing(&mut self, ui: &mut egui::Ui, status: &Status) {
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            ui.label(RichText::new(tr("Show a code here")).strong());
            ui.label(tr("Then type it on the other computer."));
            ui.add_space(6.0);
            match &status.pairing {
                Some((code, until)) => {
                    ui.label(RichText::new(code.to_string()).monospace().size(34.0).strong());
                    let left = until.saturating_duration_since(Instant::now()).as_secs();
                    let remaining = format!("{}:{:02}", left / 60, left % 60);
                    ui.label(trf("valid for {} min", &[&remaining]));
                    if ui.horizontal(|ui| ui.button(tr("Stop"))).inner.clicked() {
                        self.send(Command::StopPairing);
                    }
                }
                None => {
                    if ui.horizontal(|ui| ui.button(tr("Show pairing code"))).inner.clicked() {
                        self.send(Command::StartPairing);
                        if let Some(demo) = &mut self.demo {
                            demo.pairing =
                                tc_net::PairingCode::parse("482913").map(|c| (c, Instant::now() + PAIRING_WINDOW));
                        }
                    }
                }
            }

            let ui = &mut cols[1];
            ui.label(RichText::new(tr("Enter a code from another computer")).strong());
            let showing: Vec<_> = status.discovered.iter().filter(|d| d.pairing).collect();
            if showing.is_empty() {
                ui.label(RichText::new(tr("No computer on the network shows a code right now.")).weak());
            } else if self
                .pair_target
                .is_none_or(|t| !showing.iter().any(|d| d.fingerprint == t))
            {
                self.pair_target = showing.first().map(|d| d.fingerprint);
            }
            for d in &showing {
                ui.radio_value(&mut self.pair_target, Some(d.fingerprint), &d.name);
            }
            ui.horizontal(|ui| {
                ui.label(tr("Code"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.code_input)
                        .desired_width(90.0)
                        .hint_text("123 456"),
                );
            });
            ui.collapsing(tr("Not found? Enter the address"), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.address_input)
                        .desired_width(180.0)
                        .hint_text("192.168.1.20:24800"),
                );
            });
            let code_ok = tc_net::PairingCode::parse(&self.code_input).is_some();
            let address: Option<SocketAddr> = self.address_input.trim().parse().ok();
            let target = address
                .map(PairTarget::Address)
                .or(self.pair_target.map(PairTarget::Discovered));
            ui.horizontal(|ui| {
                let enabled = code_ok && target.is_some() && !status.pairing_busy;
                if ui.add_enabled(enabled, egui::Button::new(tr("Pair"))).clicked()
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
        for peer in &status.peers {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&peer.name).strong());
                ui.label(RichText::new(peer.fingerprint.short()).monospace().weak());
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

/// A computer that can be dragged.
fn chip(ui: &mut egui::Ui, fp: Fingerprint, name: &str, connected: bool) {
    let id = egui::Id::new(("chip", fp));
    ui.dnd_drag_source(id, Dragged(fp), |ui| {
        let color = if connected {
            Color32::from_rgb(0x2B, 0x6C, 0xD9)
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        let text_color = if connected {
            Color32::WHITE
        } else {
            ui.visuals().text_color()
        };
        let stroke = if connected {
            egui::Stroke::NONE
        } else {
            egui::Stroke::new(1.0, ui.visuals().widgets.inactive.fg_stroke.color)
        };
        egui::Frame::new()
            .fill(color)
            .stroke(stroke)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.label(RichText::new(name).color(text_color).strong());
            });
    })
    .response
    .on_hover_cursor(egui::CursorIcon::Grab);
}
