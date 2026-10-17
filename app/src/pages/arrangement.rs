//! Arrangement: the computers as tiles on the desk, dragged to where they stand. Tiles that touch
//! are neighbors where their edges overlap, also half beside each other; green bars show where the
//! pointer can cross. Computers used in turn at the same place share a tile.

use eframe::egui::{self, Color32, RichText, Vec2};
use tc_net::Fingerprint;
use tc_session::{Command, Status, layout_file};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::theme;

const TILE: Vec2 = Vec2::new(150.0, 136.0);
/// A dragged tile snaps to another one closer than this (in tiles).
const SNAP: f32 = 0.4;
/// Offsets along an edge go in these steps (in tiles).
const STEP: f32 = 0.25;
/// Tiles narrower than this (points) show no keyboard symbol and no menu dots.
const NARROW: f32 = 76.0;
/// Positions closer than this are the same.
const EPS: f32 = 1e-3;

/// A computer dragged from the list of those not placed.
#[derive(Clone)]
struct Dragged(String);

/// A computer on the desk: machine id and position in tiles.
type Placed = (String, f32, f32);

impl App {
    pub fn arrangement_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            tr("Arrangement"),
            tr("Arrange the computers like they stand on your desk."),
        );
        let status = self.status.clone();
        card(ui, tr("Desk"), |ui| self.arrangement(ui, &status));
    }

    /// The desk: computers are dragged to any place, snapping to the edges of the others. The same
    /// arrangement holds on every paired computer.
    pub fn arrangement(&mut self, ui: &mut egui::Ui, status: &Status) {
        let p = theme::colors(ui);
        ui.label(
            RichText::new(tr(
                "Drag the computers to where they stand on your desk. Where two touch, the pointer crosses \
                 (green); a computer may also stand half beside another. Drop a computer onto another one \
                 if both are used in turn at the same place.",
            ))
            .color(p.muted),
        );
        ui.add_space(12.0);
        let Some(me_fp) = status.fingerprint else { return };
        let me = layout_file::machine_id(&me_fp);
        let mut desk = layout_file::desk(&status.layout, |entry| canonical(status, &me_fp, entry));
        // This computer is always on the desk
        if !desk.iter().any(|(m, ..)| *m == me) {
            let free = (0..)
                .map(|i| i as f32)
                .find(|x| !desk.iter().any(|(_, tx, ty)| overlaps((*x, 0.0), (*tx, *ty))))
                .unwrap_or(0.0);
            desk.push((me.clone(), free, 0.0));
        }
        let order = layout_file::reading_order(&desk);
        let number = |id: &str| order.iter().position(|m| m == id).map(|i| i + 1).filter(|n| *n <= 9);
        let leader = match &status.controlled_by {
            Some(name) => status
                .peers
                .iter()
                .find(|peer| peer.name == *name)
                .map(|peer| layout_file::machine_id(&peer.fingerprint)),
            None => Some(me.clone()),
        };

        // The desk with room around it, at least four tiles wide and three high
        let (min_x, max_x) = span(desk.iter().map(|(_, x, _)| *x), 4.0);
        let (min_y, max_y) = span(desk.iter().map(|(_, _, y)| *y), 3.0);
        let cols = max_x - min_x;
        let width = ((ui.available_width() - 24.0) / cols).clamp(70.0, TILE.x);
        let tile = Vec2::new(width, width * TILE.y / TILE.x);
        let screens: Vec<(String, Vec<egui::Rect>)> = desk
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
            Some((_, s)) => Shapes { screens: s, scale },
            None => Shapes {
                screens: &unknown,
                scale: unknown_scale,
            },
        };

        let mut change: Option<Change> = None;
        let unplaced_zone_id = egui::Id::new("tc-unplaced-zone");
        egui::Frame::new()
            .fill(p.window)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let canvas_size = egui::vec2(cols * tile.x, (max_y - min_y) * tile.y);
                let (row, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), canvas_size.y), egui::Sense::hover());
                let canvas = egui::Rect::from_center_size(row.center(), canvas_size);
                let canvas_response = ui.interact(canvas, egui::Id::new("tc-desk"), egui::Sense::hover());
                let origin = canvas.min - egui::vec2(min_x * tile.x, min_y * tile.y);
                let to_screen = |x: f32, y: f32| origin + egui::vec2(x * tile.x, y * tile.y);
                let to_desk = |pos: egui::Pos2| {
                    let d = pos - origin;
                    (d.x / tile.x, d.y / tile.y)
                };
                // A faint grid helps to see where tiles stand
                for gx in (min_x as i32)..=(max_x as i32) {
                    let x = to_screen(gx as f32, 0.0).x;
                    ui.painter().vline(
                        x,
                        canvas.y_range(),
                        egui::Stroke::new(1.0, p.border.gamma_multiply(0.35)),
                    );
                }
                for gy in (min_y as i32)..=(max_y as i32) {
                    let y = to_screen(0.0, gy as f32).y;
                    ui.painter().hline(
                        canvas.x_range(),
                        y,
                        egui::Stroke::new(1.0, p.border.gamma_multiply(0.35)),
                    );
                }

                // A tile being dragged on the desk, or a computer from the list over the desk
                let pointer = ui.ctx().pointer_interact_pos();
                let drag_id = egui::Id::new("tc-dragging");
                let mut dragging: Option<(String, egui::Vec2)> = ui.ctx().data(|d| d.get_temp(drag_id));
                let from_list = egui::DragAndDrop::payload::<Dragged>(ui.ctx()).map(|d| d.0.clone());
                let moving = dragging.as_ref().map(|(m, _)| m.clone()).or(from_list.clone());
                let target = moving.as_ref().and_then(|m| {
                    let grab = dragging
                        .as_ref()
                        .map_or(egui::vec2(tile.x / 2.0, tile.y / 2.0), |(_, g)| *g);
                    let pos = pointer?;
                    canvas.expand(tile.x).contains(pos).then_some(())?;
                    let raw = to_desk(pos - grab);
                    snap(raw, m, &desk)
                });
                let shown: Vec<Placed> = desk
                    .iter()
                    .filter(|(m, ..)| Some(m) != moving.as_ref())
                    .cloned()
                    .chain(moving.iter().zip(target).map(|(m, (x, y))| (m.clone(), x, y)))
                    .collect();

                // Green bars where the pointer crosses
                for (i, a) in shown.iter().enumerate() {
                    for b in &shown[i + 1..] {
                        if let Some((from, to)) = shared_edge((a.1, a.2), (b.1, b.2)) {
                            let (from, to) = (to_screen(from.0, from.1), to_screen(to.0, to.1));
                            let inset = (to - from).normalized() * 6.0;
                            ui.painter()
                                .line_segment([from + inset, to - inset], egui::Stroke::new(4.0, p.ok));
                        }
                    }
                }

                // The tiles, those sharing a place side by side
                let mut places: Vec<(f32, f32, Vec<String>)> = Vec::new();
                for (m, x, y) in &shown {
                    match places.iter_mut().find(|(px, py, _)| same((*px, *py), (*x, *y))) {
                        Some(place) => place.2.push(m.clone()),
                        None => places.push((*x, *y, vec![m.clone()])),
                    }
                }
                for (x, y, occupants) in &places {
                    let cell = egui::Rect::from_min_size(to_screen(*x, *y), tile).shrink(4.0);
                    let alone = !shown.iter().any(|o| shared_edge((*x, *y), (o.1, o.2)).is_some());
                    let n = occupants.len() as f32;
                    let part_w = (cell.width() - 4.0 * (n - 1.0)) / n;
                    let with_me = occupants.contains(&me);
                    let frame = egui::Frame::new()
                        .fill(p.card)
                        .corner_radius(8.0)
                        .stroke(egui::Stroke::new(1.0, p.border));
                    ui.painter().add(frame.paint(cell));
                    for (i, id) in occupants.iter().enumerate() {
                        let part = egui::Rect::from_min_size(
                            cell.min + egui::vec2(i as f32 * (part_w + 4.0), 0.0),
                            egui::vec2(part_w, cell.height()),
                        );
                        let mut view = machine_view(status, &me, id);
                        view.in_turn = with_me && *id != me;
                        view.alone = alone && desk.len() > 1;
                        let ghost = Some(id) == moving.as_ref();
                        ui.scope_builder(egui::UiBuilder::new().max_rect(part), |ui| {
                            if ghost || view.alone {
                                ui.multiply_opacity(if ghost { 0.6 } else { 0.5 });
                            }
                            draw_tile(ui, part, &view, shapes(id).fitted(monitor_area(part.size())));
                            badge(ui, part, number(id));
                            if leader.as_deref() == Some(id.as_str()) && part.width() >= NARROW {
                                input_symbol(
                                    ui,
                                    part.min + egui::vec2(if number(id).is_some() { 24.0 } else { 6.0 }, 6.0),
                                );
                            }
                        });
                        if ghost {
                            continue;
                        }
                        // Dragging the tile, its menu on the dots or a right click
                        let response = ui
                            .interact(part, egui::Id::new(("tc-tile", id)), egui::Sense::click_and_drag())
                            .on_hover_cursor(egui::CursorIcon::Grab)
                            .on_hover_text(view.hover());
                        if response.drag_started()
                            && let Some(pos) = response.interact_pointer_pos()
                        {
                            dragging = Some((id.clone(), pos - to_screen(*x, *y)));
                        }
                        let dots_rect = egui::Rect::from_min_size(
                            part.right_top() + egui::vec2(-24.0, 4.0),
                            egui::vec2(20.0, 12.0),
                        );
                        let dots_response =
                            ui.interact(dots_rect, egui::Id::new(("tc-dots", id)), egui::Sense::click());
                        dots(ui, dots_rect, dots_response.hovered());
                        let menu = |ui: &mut egui::Ui| tile_menu(ui, status, &me, id);
                        if let Some(picked) = egui::Popup::menu(&dots_response).show(menu).and_then(|r| r.inner) {
                            change = Some(picked);
                        }
                        if let Some(picked) = egui::Popup::context_menu(&response).show(menu).and_then(|r| r.inner) {
                            change = Some(picked);
                        }
                    }
                }

                // Letting go
                if let Some((id, _)) = dragging.clone() {
                    if ui.input(|i| i.pointer.any_released()) || !ui.input(|i| i.pointer.any_down()) {
                        let zone: Option<egui::Rect> = ui.ctx().data(|d| d.get_temp(unplaced_zone_id));
                        if id != me && pointer.is_some_and(|pos| zone.is_some_and(|z| z.contains(pos))) {
                            change = Some(Change::Remove(id));
                        } else if let Some((x, y)) = target {
                            change = Some(Change::Move(id, x, y));
                        }
                        dragging = None;
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                }
                ui.ctx().data_mut(|d| match &dragging {
                    Some(drag) => {
                        d.insert_temp(drag_id, drag.clone());
                    }
                    None => d.remove::<(String, egui::Vec2)>(drag_id),
                });
                if let Some(payload) = canvas_response.dnd_release_payload::<Dragged>()
                    && let Some((x, y)) = target
                {
                    change = Some(Change::Move(payload.0.clone(), x, y));
                }
            });
        ui.add_space(12.0);

        // Paired computers that are not on the desk; a tile dropped here leaves the desk
        let zone = egui::Frame::new()
            .fill(p.field)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(tr("Not placed:")).small().color(p.muted));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let unplaced: Vec<_> = status
                        .peers
                        .iter()
                        .map(|peer| layout_file::machine_id(&peer.fingerprint))
                        .filter(|id| !desk.iter().any(|(c, ..)| c == id))
                        .collect();
                    if unplaced.is_empty() {
                        ui.label(RichText::new(tr("none")).color(p.muted));
                    }
                    for id in unplaced {
                        let view = machine_view(status, &me, &id);
                        ui.dnd_drag_source(egui::Id::new(("tc-unplaced", &id)), Dragged(id.clone()), |ui| {
                            let (rect, _) = ui.allocate_exact_size(SMALL_TILE, egui::Sense::hover());
                            paint_machine(ui, rect, &view.name, view.caption(), view.frame(ui), view.screen(ui));
                        })
                        .response
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text(view.hover());
                    }
                });
            });
        ui.ctx()
            .data_mut(|d| d.insert_temp(unplaced_zone_id, zone.response.rect));
        let unpaired: Vec<String> = desk
            .iter()
            .map(|(id, ..)| machine_view(status, &me, id))
            .filter(|v| !v.paired && !v.in_turn)
            .map(|v| v.name)
            .collect();
        for name in unpaired {
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
        match change {
            Some(Change::Move(id, x, y)) => {
                let mut desk: Vec<Placed> = desk.into_iter().filter(|(m, ..)| *m != id).collect();
                desk.push((id, x, y));
                self.arrange(desk);
            }
            Some(Change::Remove(id)) => self.arrange(desk.into_iter().filter(|(m, ..)| *m != id).collect()),
            Some(Change::Swap(peer, swap)) => self.send(Command::SwapCmdCtrl { peer, swap }),
            Some(Change::GoTo(peer)) => self.send(Command::SwitchTo(Some(peer))),
            None => {}
        }
    }

    fn arrange(&mut self, desk: Vec<Placed>) {
        if let Some(demo) = &mut self.demo {
            demo.layout.set_tiles(layout_file::from_desk(&desk));
        }
        self.send(Command::Arrange(desk));
    }
}

/// What a click or a drag on the desk asks for.
enum Change {
    Move(String, f32, f32),
    Remove(String),
    Swap(Fingerprint, bool),
    GoTo(Fingerprint),
}

/// The menu of a computer on the desk.
fn tile_menu(ui: &mut egui::Ui, status: &Status, me: &str, id: &str) -> Option<Change> {
    let mut change = None;
    let peer = status
        .peers
        .iter()
        .find(|peer| layout_file::machine_id(&peer.fingerprint) == id);
    if let Some(peer) = peer {
        let mut swap = peer.swap_cmd_ctrl;
        if ui
            .checkbox(&mut swap, tr("Swap ⌘ and Ctrl"))
            .on_hover_text(tr(
                "While controlling this computer from here, ⌘ acts as Ctrl and the other way round",
            ))
            .changed()
        {
            change = Some(Change::Swap(peer.fingerprint, swap));
        }
        if peer.link.is_some() && ui.button(tr("Go there")).clicked() {
            change = Some(Change::GoTo(peer.fingerprint));
        }
    }
    if id != me && ui.button(tr("Take off the desk")).clicked() {
        change = Some(Change::Remove(id.to_string()));
    }
    if id == me {
        ui.label(RichText::new(tr("This computer")).color(theme::colors(ui).muted));
    }
    change
}

/// Tiles from `lo` to `hi` (exclusive) that hold `values` with half a tile of room around them,
/// widened to at least `min` tiles on alternating sides, so the desk stays in the middle.
fn span(values: impl Iterator<Item = f32>, min: f32) -> (f32, f32) {
    let (lo, hi) = values.fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    let (lo, hi) = if lo.is_finite() { (lo, hi) } else { (0.0, 0.0) };
    let (mut lo, mut hi) = (lo - 0.5, hi + 1.5);
    let mut far = true;
    while hi - lo < min {
        if far {
            hi += 0.5;
        } else {
            lo -= 0.5;
        }
        far = !far;
    }
    (lo, hi)
}

fn same(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
}

/// Two tiles cover some of the same area (and are not at the same place).
fn overlaps(a: (f32, f32), b: (f32, f32)) -> bool {
    !same(a, b) && (a.0 - b.0).abs() < 1.0 - EPS && (a.1 - b.1).abs() < 1.0 - EPS
}

/// The part of the edges two tiles share, as two points in tiles; `None` if they do not touch.
fn shared_edge(a: (f32, f32), b: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
    let overlap = |p: f32, q: f32| (p.max(q), (p + 1.0).min(q + 1.0));
    if ((a.0 - b.0).abs() - 1.0).abs() < EPS {
        let x = a.0.max(b.0);
        let (from, to) = overlap(a.1, b.1);
        (to - from > EPS).then_some(((x, from), (x, to)))
    } else if ((a.1 - b.1).abs() - 1.0).abs() < EPS {
        let y = a.1.max(b.1);
        let (from, to) = overlap(a.0, b.0);
        (to - from > EPS).then_some(((from, y), (to, y)))
    } else {
        None
    }
}

/// Where a tile of `moving` dropped at `raw` lands: snapped to the edge of a nearby tile (in steps
/// along it) or onto it (shared place), else where it is in steps. `None` where it would cover
/// part of another tile.
fn snap(raw: (f32, f32), moving: &str, desk: &[Placed]) -> Option<(f32, f32)> {
    let others: Vec<(f32, f32)> = desk
        .iter()
        .filter(|(m, ..)| m != moving)
        .map(|(_, x, y)| (*x, *y))
        .collect();
    let step = |v: f32| (v / STEP).round() * STEP;
    let along = |v: f32, base: f32| base + step(v - base).clamp(-1.0 + STEP, 1.0 - STEP);
    let mut candidates: Vec<(f32, f32)> = Vec::new();
    for &(x, y) in &others {
        candidates.push((x, y));
        candidates.push((x + 1.0, along(raw.1, y)));
        candidates.push((x - 1.0, along(raw.1, y)));
        candidates.push((along(raw.0, x), y + 1.0));
        candidates.push((along(raw.0, x), y - 1.0));
    }
    let free = |c: (f32, f32)| !others.iter().any(|o| overlaps(c, *o));
    let distance = |c: (f32, f32)| (c.0 - raw.0).hypot(c.1 - raw.1);
    candidates
        .into_iter()
        .filter(|c| free(*c) && distance(*c) < SNAP)
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .or_else(|| Some((step(raw.0), step(raw.1))).filter(|c| free(*c)))
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
    /// Touches no other computer on the desk.
    alone: bool,
}

impl MachineView {
    fn caption(&self) -> &'static str {
        if self.this {
            tr("this computer")
        } else if self.in_turn && !self.connected {
            tr("used in turn")
        } else if !self.paired {
            tr("not paired here")
        } else if !self.connected {
            tr("not connected")
        } else if self.alone {
            tr("no neighbor")
        } else {
            // A blank caption keeps all monitors the same size
            " "
        }
    }

    fn frame(&self, ui: &egui::Ui) -> Color32 {
        let p = theme::colors(ui);
        if self.this || (self.paired && self.connected) {
            p.accent
        } else if !self.paired || self.in_turn {
            p.border
        } else {
            p.muted
        }
    }

    fn screen(&self, ui: &egui::Ui) -> Color32 {
        let p = theme::colors(ui);
        if self.this || (self.paired && self.connected) {
            p.accent_soft
        } else {
            p.field
        }
    }

    /// The full text, as narrow tiles cut it.
    fn hover(&self) -> String {
        let caption = self.caption();
        if caption.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{} – {caption}", self.name)
        }
    }
}

fn machine_view(status: &Status, me: &str, id: &str) -> MachineView {
    let view = |name: String, paired, connected, this| MachineView {
        name,
        paired,
        connected,
        this,
        in_turn: false,
        alone: false,
    };
    if id == me {
        return view(status.name.clone(), true, true, true);
    }
    match status
        .peers
        .iter()
        .find(|peer| layout_file::machine_id(&peer.fingerprint) == id)
    {
        Some(peer) => view(peer.name.clone(), true, peer.link.is_some(), false),
        None => view(
            status
                .layout
                .names
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.chars().take(8).collect()),
            false,
            false,
            false,
        ),
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

fn draw_tile(ui: &egui::Ui, rect: egui::Rect, view: &MachineView, shapes: Shapes) {
    paint_screens(
        ui,
        rect,
        &view.name,
        view.caption(),
        view.frame(ui),
        view.screen(ui),
        &shapes,
    );
}

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

/// Keyboard and mouse: the computer whose keyboard and mouse are in use right now.
fn input_symbol(ui: &egui::Ui, at: egui::Pos2) {
    let p = theme::colors(ui);
    let painter = ui.painter();
    let keyboard = egui::Rect::from_min_size(at, egui::vec2(16.0, 9.0));
    painter.rect_filled(keyboard, 2.0, p.accent);
    for row in 0..2 {
        for col in 0..4 {
            let key = egui::Rect::from_min_size(
                keyboard.min + egui::vec2(2.0 + col as f32 * 3.2, 2.0 + row as f32 * 3.0),
                egui::vec2(2.0, 1.6),
            );
            painter.rect_filled(key, 0.0, p.on_accent);
        }
    }
    let mouse = egui::Rect::from_min_size(at + egui::vec2(19.0, 0.0), egui::vec2(6.5, 9.5));
    painter.rect_filled(mouse, 3.2, p.accent);
    painter.line_segment(
        [
            mouse.center_top() + egui::vec2(0.0, 1.0),
            mouse.center() - egui::vec2(0.0, 0.5),
        ],
        egui::Stroke::new(1.0, p.on_accent),
    );
}

/// Three dots that open the menu of a computer.
fn dots(ui: &egui::Ui, rect: egui::Rect, hovered: bool) {
    let p = theme::colors(ui);
    if hovered {
        ui.painter().rect_filled(rect, 4.0, p.field);
    }
    for i in -1..=1 {
        ui.painter()
            .circle_filled(rect.center() + egui::vec2(i as f32 * 4.5, 0.0), 1.6, p.muted);
    }
}

/// The screens of a computer in logical points, from its top left corner.
struct Shapes<'a> {
    screens: &'a [egui::Rect],
    /// Pixels on the page per logical point, the same for all computers.
    scale: f32,
}

/// Room above the screens for the number, the keyboard and the menu of the computer.
const SCREENS_TOP: f32 = 20.0;

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
    use super::{shared_edge, snap, span};

    fn tile(m: &str, x: f32, y: f32) -> (String, f32, f32) {
        (m.to_string(), x, y)
    }

    #[test]
    fn the_desk_has_room_around_it() {
        assert_eq!(span([0.0].into_iter(), 4.0), (-1.5, 2.5));
        assert_eq!(span([0.0].into_iter(), 3.0), (-1.0, 2.0));
        assert_eq!(span([-1.0, 0.0, 1.0, 2.0].into_iter(), 4.0), (-1.5, 3.5));
        assert_eq!(span([0.0, 1.5].into_iter(), 3.0), (-0.5, 3.0));
    }

    #[test]
    fn dropped_tiles_snap_to_edges_in_steps() {
        let desk = vec![tile("imac", 0.0, 0.0), tile("pc", 1.0, 0.0)];
        // Near the bottom edge of the iMac, a bit to the left: half below it
        assert_eq!(snap((-0.45, 1.1), "book", &desk), Some((-0.5, 1.0)));
        // Right below the iMac
        assert_eq!(snap((0.05, 0.95), "book", &desk), Some((0.0, 1.0)));
        // Onto the PC: shares its place
        assert_eq!(snap((1.1, 0.1), "book", &desk), Some((1.0, 0.0)));
        // Far away: stays there, in steps
        assert_eq!(snap((4.1, 3.3), "book", &desk), Some((4.0, 3.25)));
        // Half over the iMac and the PC: not possible
        assert_eq!(snap((0.5, 0.0), "book", &desk), None);
        // A tile moved next to its own old place
        assert_eq!(snap((2.05, 0.0), "pc", &desk), Some((2.0, 0.0)));
    }

    #[test]
    fn green_bars_cover_the_shared_part() {
        assert_eq!(shared_edge((0.0, 0.0), (1.0, 0.0)), Some(((1.0, 0.0), (1.0, 1.0))));
        assert_eq!(shared_edge((0.0, 0.0), (-0.5, 1.0)), Some(((0.0, 1.0), (0.5, 1.0))));
        assert_eq!(shared_edge((0.0, 0.0), (1.0, 1.0)), None);
        assert_eq!(shared_edge((0.0, 0.0), (0.0, 0.0)), None);
    }
}
