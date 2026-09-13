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
use crate::theme;

const TILE: Vec2 = Vec2::new(150.0, 112.0);

/// What is dragged in the arrangement.
#[derive(Clone, Copy)]
struct Dragged(Fingerprint);

impl App {
    pub fn computers_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            tr("Computers"),
            tr("Arrange the computers on your desk and pair new ones."),
        );
        let status = self.status.clone();
        card(ui, tr("Arrangement"), |ui| self.arrangement(ui, &status));
        ui.add_space(14.0);
        card(ui, tr("Pair a computer"), |ui| self.pairing(ui, &status));
        ui.add_space(14.0);
        if !status.peers.is_empty() {
            card(ui, tr("Paired computers"), |ui| self.peer_options(ui, &status));
        }
    }

    /// This computer in the middle, the four sides around it; computers are dragged onto a side.
    pub fn arrangement(&mut self, ui: &mut egui::Ui, status: &Status) {
        let p = theme::colors(ui);
        ui.label(RichText::new(tr("Drag each computer to the side where it stands on your desk.")).color(p.muted));
        ui.add_space(12.0);
        let mut dropped: Option<(Fingerprint, Option<Side>)> = None;
        let me = status.name.clone();
        let spacing = 14.0;
        let grid_width = TILE.x * 3.0 + spacing * 2.0;
        egui::Frame::new()
            .fill(p.window)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::symmetric(12, 18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - grid_width) / 2.0).max(0.0));
                    egui::Grid::new("arrangement")
                        .spacing([spacing, spacing])
                        .show(ui, |ui| {
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
                                            paint_machine(ui, rect, &me, tr("this computer"), p.accent, p.accent_soft);
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
                });
            });
        ui.add_space(12.0);
        // Computers without a place
        let (_, payload) = ui.dnd_drop_zone::<Dragged, _>(
            egui::Frame::new()
                .fill(p.field)
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(12)),
            |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(tr("Not placed:")).small().color(p.muted));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let unplaced: Vec<_> = status.peers.iter().filter(|p| p.side.is_none()).collect();
                    if unplaced.is_empty() {
                        ui.label(RichText::new(tr("none")).color(p.muted));
                    }
                    for peer in unplaced {
                        machine_tile(ui, peer.fingerprint, &peer.name, peer.link.is_some(), SMALL_TILE);
                    }
                });
            },
        );
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
        let p = theme::colors(ui);
        let occupant = status.peers.iter().find(|peer| peer.side == Some(side));
        let (_, payload) = ui.dnd_drop_zone::<Dragged, _>(egui::Frame::new(), |ui| match occupant {
            Some(peer) => machine_tile(ui, peer.fingerprint, &peer.name, peer.link.is_some(), TILE),
            None => {
                let (rect, _) = ui.allocate_exact_size(TILE, egui::Sense::hover());
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    tr("drop here"),
                    egui::FontId::proportional(13.0),
                    p.muted,
                );
            }
        });
        payload.map(|p| p.0)
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

const SMALL_TILE: Vec2 = Vec2::new(120.0, 84.0);

/// A computer as a little monitor with its name below.
fn paint_machine(ui: &egui::Ui, rect: egui::Rect, name: &str, caption: &str, frame: Color32, screen: Color32) {
    let p = theme::colors(ui);
    let text_h = if caption.is_empty() { 20.0 } else { 34.0 };
    let width = (rect.height() - text_h) * 1.45;
    let monitor = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.min.y + (rect.height() - text_h) / 2.0),
        egui::vec2(width.min(rect.width() - 8.0), rect.height() - text_h),
    );
    theme::monitor(ui.painter(), monitor, frame, screen);
    ui.painter().text(
        egui::pos2(rect.center().x, monitor.max.y + 4.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::new(13.0, theme::semibold()),
        p.text,
    );
    if !caption.is_empty() {
        ui.painter().text(
            egui::pos2(rect.center().x, monitor.max.y + 20.0),
            egui::Align2::CENTER_TOP,
            caption,
            egui::FontId::proportional(11.5),
            p.muted,
        );
    }
}

/// A computer that can be dragged.
fn machine_tile(ui: &mut egui::Ui, fp: Fingerprint, name: &str, connected: bool, size: Vec2) {
    let id = egui::Id::new(("machine", fp));
    let p = theme::colors(ui);
    ui.dnd_drag_source(id, Dragged(fp), |ui| {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let (frame, screen) = if connected {
            (p.accent, p.accent_soft)
        } else {
            (p.muted, p.field)
        };
        let caption = if connected { "" } else { tr("not connected") };
        paint_machine(ui, rect, name, caption, frame, screen);
    })
    .response
    .on_hover_cursor(egui::CursorIcon::Grab);
}
