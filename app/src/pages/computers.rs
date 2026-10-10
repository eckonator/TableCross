//! Computers: arranging them around this one by dragging, pairing, per-computer options.

use std::net::SocketAddr;
use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Vec2};
use tc_net::{Fingerprint, PAIRING_WINDOW};
use tc_session::{Command, Notice, PairTarget, Status, layout_file};

use super::{card, ok_color, warn_color};
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::texts;
use crate::theme;

const TILE: Vec2 = Vec2::new(150.0, 112.0);

/// What is dragged in the arrangement.
#[derive(Clone)]
struct Dragged(String);

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

    /// The whole desk as a grid: computers are dragged onto any cell, those next to each other
    /// become neighbors. The same arrangement holds on every paired computer.
    pub fn arrangement(&mut self, ui: &mut egui::Ui, status: &Status) {
        let p = theme::colors(ui);
        ui.label(
            RichText::new(tr(
                "Drag the computers to where they stand on your desk. Computers side by side share that edge; \
                 computers used in turn at the same place share a field.",
            ))
            .color(p.muted),
        );
        ui.add_space(12.0);
        let Some(me_fp) = status.fingerprint else { return };
        let me = layout_file::machine_id(&me_fp);
        let cells = layout_file::desk(&status.layout, &me, |entry| canonical(status, &me_fp, entry));
        let machines = |id: &str| machine_view(status, &me, id);
        let order = layout_file::reading_order(&cells);
        let number = |id: &str| order.iter().position(|m| m == id).map(|i| i + 1).filter(|n| *n <= 9);

        // The desk in the middle with one free ring around it, at least five columns and three rows
        let shown = &cells;
        let (min_x, max_x) = centered_range(shown.iter().map(|(_, x, _)| *x), 5);
        let (min_y, max_y) = centered_range(shown.iter().map(|(_, _, y)| *y), 3);
        let cols = (max_x - min_x + 1) as f32;
        let gap = 10.0;
        let width = ((ui.available_width() - 24.0 - gap * (cols - 1.0)) / cols).clamp(70.0, TILE.x);
        let tile = Vec2::new(width, width * 0.75);
        // Real screens, all at one scale, so their sizes compare
        let screens: Vec<(String, Vec<egui::Rect>)> = shown
            .iter()
            .filter_map(|(id, ..)| screens_of(status, &me, id).map(|s| (id.clone(), s)))
            .collect();
        let scale = screens
            .iter()
            .filter_map(|(_, s)| fit_scale(s, monitor_area(tile)))
            .fold(f32::INFINITY, f32::min);
        // Computers whose screens are not known (not connected) as a common full HD screen
        let unknown = [egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))];
        let unknown_scale = fit_scale(&unknown, monitor_area(tile)).unwrap_or(1.0).min(scale);
        let shapes = |id: &str| match screens.iter().find(|(m, _)| m == id) {
            Some((_, s)) => Some(Shapes { screens: s, scale }),
            None => Some(Shapes {
                screens: &unknown,
                scale: unknown_scale,
            }),
        };
        let dragging = egui::DragAndDrop::has_any_payload(ui.ctx());
        let mut dropped: Option<(String, Option<(i32, i32)>)> = None;
        let mut unreachable = Vec::new();
        egui::Frame::new()
            .fill(p.window)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let grid_width = tile.x * cols + gap * (cols - 1.0);
                    ui.add_space(((ui.available_width() - grid_width) / 2.0).max(0.0));
                    egui::Grid::new("desk").spacing([gap, gap]).show(ui, |ui| {
                        for y in min_y..=max_y {
                            for x in min_x..=max_x {
                                let occupants: Vec<&String> = shown
                                    .iter()
                                    .filter(|(_, cx, cy)| (*cx, *cy) == (x, y))
                                    .map(|(id, ..)| id)
                                    .collect();
                                let frame = egui::Frame::new()
                                    .corner_radius(8.0)
                                    .stroke(egui::Stroke::new(1.0, if dragging { p.accent } else { p.border }));
                                let (_, payload) = ui.dnd_drop_zone::<Dragged, _>(frame, |ui| match occupants.len() {
                                    1.. => {
                                        // Computers sharing the place (used in turn) side by side
                                        let n = occupants.len() as f32;
                                        let part = Vec2::new((tile.x - 4.0 * (n - 1.0)) / n, tile.y);
                                        let (cell, _) = ui.allocate_exact_size(tile, egui::Sense::hover());
                                        // Sharing the place with this computer: never side by side, no pairing needed
                                        let with_me = occupants.iter().any(|id| **id == me);
                                        for (i, id) in occupants.iter().enumerate() {
                                            let mut view = machines(id);
                                            view.in_turn = with_me && **id != me;
                                            if !view.paired && !with_me {
                                                unreachable.push(view.name.clone());
                                            }
                                            let shapes = shapes(id).map(|s| s.fitted(monitor_area(part)));
                                            let min = cell.min + egui::vec2(i as f32 * (part.x + 4.0), 0.0);
                                            let place = egui::Rect::from_min_size(min, part);
                                            ui.scope_builder(egui::UiBuilder::new().max_rect(place), |ui| {
                                                machine_tile(ui, id, &view, part, number(id), shapes);
                                            });
                                        }
                                    }
                                    _ => {
                                        let (rect, _) = ui.allocate_exact_size(tile, egui::Sense::hover());
                                        if dragging {
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                tr("drop here"),
                                                egui::FontId::proportional(12.0),
                                                p.muted,
                                            );
                                        }
                                    }
                                });
                                if let Some(payload) = payload {
                                    dropped = Some((payload.0.clone(), Some((x, y))));
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        ui.add_space(12.0);
        // Paired computers that are not on the desk
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
                    let unplaced: Vec<_> = status
                        .peers
                        .iter()
                        .map(|peer| layout_file::machine_id(&peer.fingerprint))
                        .filter(|id| !cells.iter().any(|(c, ..)| c == id))
                        .collect();
                    if unplaced.is_empty() {
                        ui.label(RichText::new(tr("none")).color(p.muted));
                    }
                    for id in unplaced {
                        machine_tile(ui, &id, &machines(&id), SMALL_TILE, None, None);
                    }
                });
            },
        );
        if let Some(payload) = payload {
            dropped = Some((payload.0.clone(), None));
        }
        for name in unreachable {
            ui.add_space(6.0);
            theme::note(
                ui,
                theme::Tone::Info,
                &trf(
                    "To control {} with this keyboard, pair it with this computer as well.",
                    &[&name],
                ),
            );
        }
        match dropped {
            // This computer stays on the desk; dropped elsewhere it changes places like the others
            Some((id, None)) if id == me => {}
            // Moved away from all others, this computer would leave them behind: not allowed
            Some((id, cell)) if id == me => {
                let moved = rearranged(&cells, &me, &id, cell);
                if attached(&moved, &me) == attached(&cells, &me) {
                    self.arrange(moved);
                }
            }
            Some((id, cell)) => self.arrange(rearranged(&cells, &me, &id, cell)),
            None => {}
        }
    }

    fn arrange(&mut self, cells: Vec<(String, i32, i32)>) {
        if let Some(demo) = &mut self.demo
            && let Some(fp) = demo.fingerprint
        {
            demo.layout.neighbors = layout_file::from_grid(&cells, &layout_file::machine_id(&fp));
        }
        self.send(Command::Arrange(cells));
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

/// How a computer on the desk is shown.
struct MachineView {
    name: String,
    paired: bool,
    connected: bool,
    this: bool,
    /// Shares the place with this computer (used in turn, never side by side).
    in_turn: bool,
}

fn machine_view(status: &Status, me: &str, id: &str) -> MachineView {
    if id == me {
        return MachineView {
            name: status.name.clone(),
            paired: true,
            connected: true,
            this: true,
            in_turn: false,
        };
    }
    match status
        .peers
        .iter()
        .find(|peer| layout_file::machine_id(&peer.fingerprint) == id)
    {
        Some(peer) => MachineView {
            name: peer.name.clone(),
            paired: true,
            connected: peer.link.is_some(),
            this: false,
            in_turn: false,
        },
        None => MachineView {
            name: status
                .layout
                .names
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.chars().take(8).collect()),
            paired: false,
            connected: false,
            this: false,
            in_turn: false,
        },
    }
}

/// One id per machine for an entry of the layout file (names and fingerprint prefixes allowed).
fn canonical(status: &Status, me: &Fingerprint, entry: &str) -> String {
    let lower = entry.to_lowercase();
    let matches = |fp: &Fingerprint, name: &str| {
        name.to_lowercase() == lower || (lower.len() >= 8 && fp.to_string().starts_with(&lower))
    };
    if matches(me, &status.name) {
        return layout_file::machine_id(me);
    }
    status
        .peers
        .iter()
        .find(|p| matches(&p.fingerprint, &p.name))
        .map_or_else(|| entry.to_string(), |p| layout_file::machine_id(&p.fingerprint))
}

/// The desk after `id` was dropped on `cell` (`None`: taken off the desk). On a cell that is taken
/// it shares the place with the computers there (used in turn); this computer's place is not shared.
/// Grid lines `values` need with one free line on each side, widened to at least `min` lines,
/// alternately on the far and the near side so the desk stays in the middle.
fn centered_range(values: impl Iterator<Item = i32>, min: i32) -> (i32, i32) {
    let (lo, hi) = values.fold((0, 0), |(a, b), v| (a.min(v), b.max(v)));
    let (mut lo, mut hi) = (lo - 1, hi + 1);
    while hi - lo + 1 < min {
        if (hi - lo) % 2 == 0 {
            hi += 1;
        } else {
            lo -= 1;
        }
    }
    (lo, hi)
}

/// How many computers hang together with `me` (directly or through others).
fn attached(cells: &[(String, i32, i32)], me: &str) -> usize {
    let mut ids: Vec<String> = layout_file::from_grid(cells, me)
        .into_iter()
        .flat_map(|n| [n.machine, n.of])
        .collect();
    ids.sort();
    ids.dedup();
    ids.len()
}

fn rearranged(cells: &[(String, i32, i32)], me: &str, id: &str, cell: Option<(i32, i32)>) -> Vec<(String, i32, i32)> {
    let mut out: Vec<_> = cells.iter().filter(|(c, ..)| c != id).cloned().collect();
    match cell {
        Some((x, y)) if out.iter().any(|(c, cx, cy)| c == me && (*cx, *cy) == (x, y)) => cells.to_vec(),
        Some((x, y)) => {
            out.push((id.to_string(), x, y));
            out
        }
        None => out,
    }
}

/// A computer that can be dragged.
/// The number of a computer in the corner of its tile (Ctrl+Shift+Alt+number goes there).
fn badge(ui: &egui::Ui, rect: egui::Rect, number: Option<usize>) {
    let Some(number) = number else { return };
    let p = theme::colors(ui);
    let center = rect.min + egui::vec2(11.0, 11.0);
    ui.painter().circle_filled(center, 8.0, p.field);
    ui.painter()
        .circle_stroke(center, 8.0, egui::Stroke::new(1.0, p.border));
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        number.to_string(),
        egui::FontId::new(11.0, theme::semibold()),
        p.muted,
    );
}

fn machine_tile(
    ui: &mut egui::Ui,
    machine: &str,
    view: &MachineView,
    size: Vec2,
    number: Option<usize>,
    shapes: Option<Shapes>,
) {
    let id = egui::Id::new(("machine", machine));
    let p = theme::colors(ui);
    let (frame, screen, caption) = if view.this {
        (p.accent, p.accent_soft, tr("this computer"))
    } else if view.in_turn && !view.connected {
        (p.border, p.field, tr("used in turn"))
    } else if !view.paired {
        (p.border, p.field, tr("not paired here"))
    } else if view.connected {
        // A blank caption keeps all monitors the same size
        (p.accent, p.accent_soft, " ")
    } else {
        (p.muted, p.field, tr("not connected"))
    };
    // The full text, as narrow tiles cut it
    let hover = if caption.trim().is_empty() {
        view.name.clone()
    } else {
        format!("{} – {caption}", view.name)
    };
    ui.dnd_drag_source(id, Dragged(machine.to_string()), |ui| {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        match shapes {
            Some(shapes) => paint_screens(ui, rect, &view.name, caption, frame, screen, &shapes),
            None => paint_machine(ui, rect, &view.name, caption, frame, screen),
        }
        badge(ui, rect, number);
    })
    .response
    .on_hover_cursor(egui::CursorIcon::Grab)
    .on_hover_text(hover);
}

/// The screens of a computer in logical points, from its top left corner.
struct Shapes<'a> {
    screens: &'a [egui::Rect],
    /// Pixels on the page per logical point, the same for all computers.
    scale: f32,
}

/// Room above the screens for the number of the computer.
const SCREENS_TOP: f32 = 8.0;

impl Shapes<'_> {
    /// Smaller if the screens do not fit into `area` (a shared place).
    fn fitted(self, area: Vec2) -> Self {
        let scale = fit_scale(self.screens, area).map_or(self.scale, |fit| fit.min(self.scale));
        Shapes { scale, ..self }
    }
}

/// Where the screens are drawn in a tile (the name goes below).
fn monitor_area(tile: Vec2) -> Vec2 {
    Vec2::new(tile.x - 12.0, tile.y - 34.0 - SCREENS_TOP - 4.0)
}

fn fit_scale(screens: &[egui::Rect], area: Vec2) -> Option<f32> {
    let bounds = screens.iter().copied().reduce(|a, b| a.union(b))?;
    (bounds.width() > 0.0 && bounds.height() > 0.0).then(|| (area.x / bounds.width()).min(area.y / bounds.height()))
}

/// The screens of this computer or of a connected peer, in logical points.
fn screens_of(status: &Status, me: &str, id: &str) -> Option<Vec<egui::Rect>> {
    let screens = if id == me {
        status.screens.clone()
    } else {
        status
            .peers
            .iter()
            .find(|peer| layout_file::machine_id(&peer.fingerprint) == id)?
            .link
            .as_ref()?
            .screens
            .clone()
    };
    let rects: Vec<egui::Rect> = screens
        .iter()
        .filter(|s| s.width > 0 && s.height > 0 && s.scale > 0.0)
        .map(|s| {
            egui::Rect::from_min_size(
                egui::pos2(s.x as f32 / s.scale, s.y as f32 / s.scale),
                egui::vec2(s.width as f32 / s.scale, s.height as f32 / s.scale),
            )
        })
        .collect();
    let bounds = rects.iter().copied().reduce(|a, b| a.union(b))?;
    Some(rects.iter().map(|r| r.translate(-bounds.min.to_vec2())).collect())
}

/// A computer drawn with its real screens (sizes to scale), a stand under each, the name below.
fn paint_screens(
    ui: &egui::Ui,
    rect: egui::Rect,
    name: &str,
    caption: &str,
    frame: Color32,
    fill: Color32,
    shapes: &Shapes,
) {
    let p = theme::colors(ui);
    let area = monitor_area(rect.size());
    let bounds = shapes
        .screens
        .iter()
        .copied()
        .reduce(|a, b| a.union(b))
        .unwrap_or(egui::Rect::ZERO);
    let size = bounds.size() * shapes.scale;
    // Bottom aligned, so screens of different sizes stand on the same desk
    let origin = egui::pos2(
        rect.center().x - size.x / 2.0,
        rect.min.y + SCREENS_TOP + area.y - size.y,
    );
    let painter = ui.painter();
    for screen in shapes.screens {
        let r = egui::Rect::from_min_size(
            origin + screen.min.to_vec2() * shapes.scale,
            screen.size() * shapes.scale,
        )
        .shrink(1.0);
        let stand = (r.height() * 0.12).clamp(3.0, 8.0);
        let body = egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.max.y - stand));
        painter.rect_filled(body, 4.0, frame);
        painter.rect_filled(body.shrink(2.5), 2.5, fill);
        let neck = egui::Rect::from_center_size(
            egui::pos2(body.center().x, body.max.y + stand / 2.0),
            egui::vec2((r.width() * 0.08).max(3.0), stand),
        );
        painter.rect_filled(neck, 0.0, frame);
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(body.center().x, r.max.y - 1.0),
                egui::vec2((r.width() * 0.3).max(10.0), 2.5),
            ),
            1.0,
            frame,
        );
    }
    let text_top = rect.min.y + SCREENS_TOP + area.y + 4.0;
    // Long names end in "…" (shared places are narrow)
    let line = |text: &str, font: egui::FontId, color: Color32, top: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 6.0);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, top);
        painter.galley(pos, galley, color);
    };
    line(name, egui::FontId::new(13.0, theme::semibold()), p.text, text_top);
    if !caption.trim().is_empty() {
        line(caption, egui::FontId::proportional(11.5), p.muted, text_top + 16.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{attached, centered_range, rearranged};

    #[test]
    fn the_desk_is_centered_with_room_around_it() {
        // Only this computer: five by three around it
        assert_eq!(centered_range([0].into_iter(), 5), (-2, 2));
        assert_eq!(centered_range([0].into_iter(), 3), (-1, 1));
        // Wide desk: one free line on each side, nothing more
        assert_eq!(centered_range([-1, 0, 1, 2].into_iter(), 5), (-2, 3));
        // Desk above this computer: rows from it, not from this computer's place
        assert_eq!(centered_range([-1, 0].into_iter(), 3), (-2, 1));
    }

    #[test]
    fn this_computer_cannot_leave_the_others_behind() {
        let desk = vec![cell("me", 0, 0), cell("imac", 0, -1), cell("book2", -1, -1)];
        assert_eq!(attached(&desk, "me"), 3);
        let away = rearranged(&desk, "me", "me", Some((2, 2)));
        assert_eq!(attached(&away, "me"), 0);
        let beside = rearranged(&desk, "me", "me", Some((1, -1)));
        assert_eq!(attached(&beside, "me"), 3);
    }

    fn cell(m: &str, x: i32, y: i32) -> (String, i32, i32) {
        (m.to_string(), x, y)
    }

    #[test]
    fn dropping_moves_shares_and_removes() {
        let desk = vec![cell("me", 0, 0), cell("pc", 1, 0), cell("pi", -1, 0)];
        // Onto a free cell
        let moved = rearranged(&desk, "me", "pi", Some((2, 0)));
        assert!(moved.contains(&cell("pi", 2, 0)) && moved.len() == 3);
        // Onto another computer: both share the place
        let shared = rearranged(&desk, "me", "pi", Some((1, 0)));
        assert!(shared.contains(&cell("pi", 1, 0)) && shared.contains(&cell("pc", 1, 0)));
        // Onto this computer: nothing changes
        assert_eq!(rearranged(&desk, "me", "pi", Some((0, 0))), desk);
        // From outside onto a taken cell
        let new = rearranged(&desk, "me", "tv", Some((1, 0)));
        assert!(new.contains(&cell("tv", 1, 0)) && new.contains(&cell("pc", 1, 0)));
        // Off the desk
        let off = rearranged(&desk, "me", "pc", None);
        assert_eq!(off, vec![cell("me", 0, 0), cell("pi", -1, 0)]);
    }
}
