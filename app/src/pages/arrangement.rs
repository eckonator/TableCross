//! Arrangement: the computers as areas of their real size on the desk, dragged to where they stand.
//! Areas that touch are neighbors where their edges overlap, also offset; green bars show where the
//! pointer crosses, on the page and, while dragging, at the real screen edges of every computer.
//! Computers used in turn at the same place share an area.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Vec2};
use tc_core::{Area, STAND_IN};
use tc_net::Fingerprint;
use tc_protocol::Screen;
use tc_session::{Command, Status, layout_file, screen_size};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::theme;

/// Within this distance (points on the page) a dragged computer snaps to an edge or a corner.
const SNAP_PX: f32 = 28.0;
/// Most points on the page per thousand points of screen (a full HD screen at most this wide × 1.92).
const MAX_SCALE: f32 = 150.0;
/// Areas narrower than this (points) show no keyboard symbol and no menu dots.
const NARROW: f32 = 76.0;
/// Positions closer than this are the same (thousands of points).
const EPS: f32 = 1e-3;
/// How often the bars on the other computers are refreshed while dragging.
const EDGES_EVERY: Duration = Duration::from_millis(150);

/// A computer dragged from the list of those not placed.
#[derive(Clone)]
struct Dragged(String);

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
                "Drag the computers to where they stand on your desk, in their real size. Where two touch, the \
                 pointer crosses (green); while dragging, the screens show it at their edges as well. Drop a \
                 computer onto another one if both are used in turn at the same place.",
            ))
            .color(p.muted),
        );
        ui.add_space(12.0);
        let Some(me_fp) = status.fingerprint else { return };
        let me = layout_file::machine_id(&me_fp);
        let mut desk = layout_file::desk(&status.layout, |entry| canonical(status, &me_fp, entry));
        // This computer is always on the desk
        if !desk.iter().any(|a| a.machine == me) {
            let (w, h) = real_size(status, &me, &me).unwrap_or(STAND_IN);
            let x = desk.iter().map(|a| a.x + a.w).fold(0.0f32, f32::max);
            desk.push(Area {
                machine: me.clone(),
                x: if desk.is_empty() { 0.0 } else { x + 0.5 },
                y: 0.0,
                w,
                h,
                measured: true,
            });
        }
        self.measure(ui.ctx(), status, &me, &desk);
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

        let mut change: Option<Change> = None;
        let unplaced_zone_id = egui::Id::new("tc-unplaced-zone");
        let drag_id = egui::Id::new("tc-dragging");
        let mut dragging: Option<(String, Vec2)> = ui.ctx().data(|d| d.get_temp(drag_id));
        let from_list = egui::DragAndDrop::payload::<Dragged>(ui.ctx()).map(|d| d.0.clone());
        let moving = dragging.as_ref().map(|(m, _)| m.clone()).or(from_list.clone());
        let mut shown_desk: Vec<Area> = desk.clone();
        egui::Frame::new()
            .fill(p.window)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let height = (ui.available_width() * 0.6).clamp(340.0, 700.0);
                let (canvas, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
                let canvas_response = ui.interact(canvas, egui::Id::new("tc-desk"), egui::Sense::hover());
                // Zoom so the desk fills the room, with space around it to drop computers
                let lo = desk.iter().fold(egui::pos2(f32::INFINITY, f32::INFINITY), |m, a| {
                    egui::pos2(m.x.min(a.x), m.y.min(a.y))
                });
                let hi = desk
                    .iter()
                    .fold(egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY), |m, a| {
                        egui::pos2(m.x.max(a.x + a.w), m.y.max(a.y + a.h))
                    });
                let room = (hi - lo) + egui::vec2(STAND_IN.0, STAND_IN.1) * 0.6;
                let scale = (canvas.width() / room.x).min(canvas.height() / room.y).min(MAX_SCALE);
                let middle = lo + (hi - lo) / 2.0;
                let origin = canvas.center() - middle.to_vec2() * scale;
                let to_screen = |x: f32, y: f32| origin + egui::vec2(x * scale, y * scale);
                let to_desk = |pos: egui::Pos2| {
                    let d = pos - origin;
                    (d.x / scale, d.y / scale)
                };

                // A computer being dragged on the desk, or one from the list over the desk
                let pointer = ui.ctx().pointer_interact_pos();
                let target = moving.as_ref().and_then(|m| {
                    let size = desk
                        .iter()
                        .find(|a| a.machine == *m)
                        .map(|a| (a.w, a.h))
                        .or_else(|| real_size(status, &me, m))
                        .unwrap_or(STAND_IN);
                    let grab = dragging
                        .as_ref()
                        .map_or(egui::vec2(size.0, size.1) * scale / 2.0, |(_, g)| *g);
                    let pos = pointer?;
                    canvas.expand(40.0).contains(pos).then_some(())?;
                    let raw = to_desk(pos - grab);
                    snap(raw, size, m, &desk, SNAP_PX / scale).map(|(x, y)| (x, y, size))
                });
                if let (Some(m), Some((x, y, (w, h)))) = (&moving, target) {
                    let measured = desk.iter().find(|a| a.machine == *m).is_none_or(|a| a.measured);
                    shown_desk.retain(|a| a.machine != *m);
                    shown_desk.push(Area {
                        machine: m.clone(),
                        x,
                        y,
                        w,
                        h,
                        measured,
                    });
                }
                let shown = &shown_desk;

                // Green bars where the pointer crosses
                for (i, a) in shown.iter().enumerate() {
                    for b in &shown[i + 1..] {
                        if let Some((from, to)) = shared_edge(a, b) {
                            let (from, to) = (to_screen(from.0, from.1), to_screen(to.0, to.1));
                            let inset = (to - from).normalized() * 4.0;
                            ui.painter()
                                .line_segment([from + inset, to - inset], egui::Stroke::new(4.0, p.ok));
                        }
                    }
                }

                // The computers, those sharing a place side by side
                let mut places: Vec<(Area, Vec<String>)> = Vec::new();
                for a in shown {
                    match places.iter_mut().find(|(o, _)| same(o, a)) {
                        Some(place) => place.1.push(a.machine.clone()),
                        None => places.push((a.clone(), vec![a.machine.clone()])),
                    }
                }
                for (area, occupants) in &places {
                    let cell = egui::Rect::from_min_size(to_screen(area.x, area.y), egui::vec2(area.w, area.h) * scale)
                        .shrink(3.0);
                    let alone = !shown.iter().any(|o| shared_edge(area, o).is_some());
                    let n = occupants.len() as f32;
                    let part_w = (cell.width() - 4.0 * (n - 1.0)) / n;
                    let with_me = occupants.contains(&me);
                    for (i, id) in occupants.iter().enumerate() {
                        let part = egui::Rect::from_min_size(
                            cell.min + egui::vec2(i as f32 * (part_w + 4.0), 0.0),
                            egui::vec2(part_w, cell.height()),
                        );
                        let mut view = machine_view(status, &me, id);
                        view.in_turn = with_me && *id != me;
                        view.alone = alone && shown.len() > 1;
                        let ghost = Some(id) == moving.as_ref();
                        ui.scope_builder(egui::UiBuilder::new().max_rect(part), |ui| {
                            if ghost || view.alone {
                                ui.multiply_opacity(if ghost { 0.7 } else { 0.55 });
                            }
                            draw_computer(ui, part, &view, screens_of(status, &me, id).as_deref());
                            badge(ui, part, number(id));
                            if leader.as_deref() == Some(id.as_str()) && part.width() >= NARROW {
                                let x = if number(id).is_some() { 24.0 } else { 6.0 };
                                input_symbol(ui, part.min + egui::vec2(x, 6.5));
                            }
                        });
                        if ghost {
                            continue;
                        }
                        // Dragging, the menu on the dots or a right click
                        let response = ui
                            .interact(part, egui::Id::new(("tc-tile", id)), egui::Sense::click_and_drag())
                            .on_hover_cursor(egui::CursorIcon::Grab)
                            .on_hover_text(view.hover());
                        if response.drag_started()
                            && let Some(pos) = response.interact_pointer_pos()
                        {
                            dragging = Some((id.clone(), pos - to_screen(area.x, area.y)));
                        }
                        let menu = |ui: &mut egui::Ui| tile_menu(ui, status, &me, id);
                        if part.width() >= NARROW {
                            let dots_rect = egui::Rect::from_min_size(
                                part.right_top() + egui::vec2(-24.0, 4.0),
                                egui::vec2(20.0, 12.0),
                            );
                            let dots_response =
                                ui.interact(dots_rect, egui::Id::new(("tc-dots", id)), egui::Sense::click());
                            dots(ui, dots_rect, dots_response.hovered());
                            if let Some(picked) = egui::Popup::menu(&dots_response).show(menu).and_then(|r| r.inner) {
                                change = Some(picked);
                            }
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
                        } else if let Some((x, y, _)) = target {
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
                    None => d.remove::<(String, Vec2)>(drag_id),
                });
                if let Some(payload) = canvas_response.dnd_release_payload::<Dragged>()
                    && let Some((x, y, _)) = target
                {
                    change = Some(Change::Move(payload.0.clone(), x, y));
                }
            });
        self.live_edges(ui.ctx(), status, &me, moving.is_some().then_some(&shown_desk));
        ui.add_space(12.0);

        // Paired computers that are not on the desk; a computer dropped here leaves the desk
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
                        .filter(|id| !desk.iter().any(|a| a.machine == *id))
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
            .map(|a| machine_view(status, &me, &a.machine))
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
        if status.size_unsure {
            ui.add_space(6.0);
            self.diagonal(ui);
        }
        match change {
            Some(Change::Move(id, x, y)) => {
                let (w, h, measured) = desk
                    .iter()
                    .find(|a| a.machine == id)
                    .map(|a| (a.w, a.h, a.measured))
                    .or_else(|| real_size(status, &me, &id).map(|(w, h)| (w, h, true)))
                    .unwrap_or((STAND_IN.0, STAND_IN.1, false));
                let mut desk: Vec<Area> = desk.into_iter().filter(|a| a.machine != id).collect();
                desk.push(Area {
                    machine: id,
                    x,
                    y,
                    w,
                    h,
                    measured,
                });
                self.arrange(desk);
            }
            Some(Change::Remove(id)) => self.arrange(desk.into_iter().filter(|a| a.machine != id).collect()),
            Some(Change::Resize(id)) => {
                if let Some(size) = real_size(status, &me, &id) {
                    let resized = tc_core::resized(&desk, |a| (a.machine == id).then_some(size), &me);
                    let resized = resized
                        .into_iter()
                        .map(|a| Area {
                            measured: a.measured || a.machine == id,
                            ..a
                        })
                        .collect();
                    self.arrange(resized);
                }
            }
            Some(Change::Swap(peer, swap)) => self.send(Command::SwapCmdCtrl { peer, swap }),
            Some(Change::GoTo(peer)) => self.send(Command::SwitchTo(Some(peer))),
            None => {}
        }
    }

    /// Computers still shown in a stand-in size get their real one as soon as their screens are
    /// known; the desk keeps its shape around this computer. Later on each computer measures
    /// itself when its screens change.
    fn measure(&mut self, ctx: &egui::Context, status: &Status, me: &str, desk: &[Area]) {
        let sizes: Vec<(String, (f32, f32))> = desk
            .iter()
            .filter(|a| !a.measured)
            .filter_map(|a| real_size(status, me, &a.machine).map(|s| (a.machine.clone(), s)))
            .collect();
        if sizes.is_empty() {
            return;
        }
        // Once per arrangement version: the new one takes a moment to come back
        let id = egui::Id::new("tc-measured-revision");
        let done: Option<u64> = ctx.data(|d| d.get_temp(id));
        if done == Some(status.layout.revision) {
            return;
        }
        ctx.data_mut(|d| d.insert_temp(id, status.layout.revision));
        // Computers sharing a place use the same screens: they get the size as well
        let size = |a: &Area| {
            let of = |m: &str| sizes.iter().find(|(s, _)| s == m).map(|(_, s)| *s);
            of(&a.machine).or_else(|| desk.iter().filter(|b| same(a, b)).find_map(|b| of(&b.machine)))
        };
        let resized = tc_core::resized(desk, size, me)
            .into_iter()
            .map(|a| Area {
                measured: a.measured || sizes.iter().any(|(m, _)| *m == a.machine),
                ..a
            })
            .collect();
        tracing::info!("arrangement: real sizes of {} computer(s) taken over", sizes.len());
        self.arrange(resized);
    }

    /// While a computer is dragged: green bars at the real screen edges here and, through the
    /// network, on the other computers. `None`: not dragging (the bars go away).
    fn live_edges(&mut self, ctx: &egui::Context, status: &Status, me: &str, shown: Option<&Vec<Area>>) {
        let id = egui::Id::new("tc-edges-sent");
        let sent: Option<Instant> = ctx.data(|d| d.get_temp(id));
        let Some(shown) = shown else {
            if sent.is_some() {
                // Dragging ended: away with the bars everywhere
                let clear = status.peers.iter().map(|p| (p.fingerprint, Vec::new())).collect();
                self.send(Command::ShowEdges(clear));
                ctx.data_mut(|d| d.remove::<Instant>(id));
            }
            return;
        };
        let marks = crate::edges::marks(shown);
        if let Some((_, mine)) = marks.iter().find(|(m, _)| m == me) {
            crate::edges::show(ctx, &status.screens, mine, theme::palette(ctx).ok);
        }
        if sent.is_none_or(|t| t.elapsed() >= EDGES_EVERY) {
            let to_peers = status
                .peers
                .iter()
                .filter(|p| p.link.is_some())
                .map(|p| {
                    let id = layout_file::machine_id(&p.fingerprint);
                    let marks = marks
                        .iter()
                        .find(|(m, _)| *m == id)
                        .map(|(_, marks)| marks.clone())
                        .unwrap_or_default();
                    (p.fingerprint, marks)
                })
                .collect();
            self.send(Command::ShowEdges(to_peers));
            ctx.data_mut(|d| d.insert_temp(id, Instant::now()));
        }
        ctx.request_repaint_after(EDGES_EVERY);
    }

    /// A screen here reports a size that cannot be right: its diagonal can be typed in. Without
    /// it, the size of a screen with the same points on another computer counts, else the points.
    fn diagonal(&mut self, ui: &mut egui::Ui) {
        theme::note(
            ui,
            theme::Tone::Info,
            tr(
                "A screen of this computer reports a size that cannot be right. Without its diagonal it counts \
                as large as a screen with the same resolution on another computer.",
            ),
        );
        ui.add_space(4.0);
        let mut by_hand = self.settings.screen_diagonal.is_some();
        let mut inches = self.settings.screen_diagonal.unwrap_or(27.0);
        let edited = ui
            .horizontal(|ui| {
                let mut edited = ui.checkbox(&mut by_hand, tr("Its diagonal:")).changed();
                edited |= ui
                    .add_enabled(
                        by_hand,
                        egui::DragValue::new(&mut inches)
                            .range(5.0..=150.0)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(tr(" inches")),
                    )
                    .changed();
                edited
            })
            .inner;
        if edited {
            self.settings.screen_diagonal = by_hand.then_some(inches);
            self.save_settings();
            self.send(Command::ScreenDiagonal(self.settings.screen_diagonal));
        }
    }

    fn arrange(&mut self, desk: Vec<Area>) {
        if let Some(demo) = &mut self.demo {
            demo.layout.set_areas(desk.clone());
        }
        self.send(Command::Arrange(desk));
    }
}

/// What a click or a drag on the desk asks for.
enum Change {
    Move(String, f32, f32),
    Remove(String),
    Resize(String),
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
    if real_size(status, me, id).is_some()
        && ui
            .button(tr("Take over the current size"))
            .on_hover_text(tr(
                "Takes the screens it has now; each computer also does this by itself when its screens change.",
            ))
            .clicked()
    {
        change = Some(Change::Resize(id.to_string()));
    }
    if id != me && ui.button(tr("Take off the desk")).clicked() {
        change = Some(Change::Remove(id.to_string()));
    }
    change
}

/// The size of a computer from its screens, in thousands of points (`None` while not known).
fn real_size(status: &Status, me: &str, id: &str) -> Option<(f32, f32)> {
    let screens = screens_of(status, me, id)?;
    let bounds = screens.iter().copied().reduce(|a, b| a.union(b))?;
    Some((bounds.width() / 1000.0, bounds.height() / 1000.0))
}

fn same(a: &Area, b: &Area) -> bool {
    (a.x - b.x).abs() < EPS && (a.y - b.y).abs() < EPS
}

/// Two areas cover some of the same space (and are not at the same place).
fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    let same = (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS;
    !same && a.0 < b.0 + b.2 - EPS && b.0 < a.0 + a.2 - EPS && a.1 < b.1 + b.3 - EPS && b.1 < a.1 + a.3 - EPS
}

/// The part of the edges two areas share, as two points; `None` if they do not touch.
fn shared_edge(a: &Area, b: &Area) -> Option<((f32, f32), (f32, f32))> {
    let side = tc_core::area_side(a, b)?;
    Some(match side {
        tc_core::Side::Left | tc_core::Side::Right => {
            let x = if side == tc_core::Side::Right { a.x + a.w } else { a.x };
            ((x, a.y.max(b.y)), (x, (a.y + a.h).min(b.y + b.h)))
        }
        tc_core::Side::Top | tc_core::Side::Bottom => {
            let y = if side == tc_core::Side::Bottom { a.y + a.h } else { a.y };
            ((a.x.max(b.x), y), ((a.x + a.w).min(b.x + b.w), y))
        }
    })
}

/// Where a computer of `size` dropped at `raw` lands: at an edge of a nearby computer, aligned with
/// its corners or middle when close, onto it (shared place), or else where it is. `None` where it
/// would cover part of another computer. `reach`: snapping distance in thousands of points.
fn snap(raw: (f32, f32), size: (f32, f32), moving: &str, desk: &[Area], reach: f32) -> Option<(f32, f32)> {
    let (w, h) = size;
    let others: Vec<(f32, f32, f32, f32)> = desk
        .iter()
        .filter(|a| a.machine != moving)
        .map(|a| (a.x, a.y, a.w, a.h))
        .collect();
    // Along an edge: aligned with a corner or the middle when close, else free (keeping some overlap)
    let along = |v: f32, start: f32, len: f32, own: f32| {
        let aligned = [start, start + len - own, start + (len - own) / 2.0];
        aligned
            .into_iter()
            .find(|a| (a - v).abs() < reach)
            .unwrap_or_else(|| v.clamp(start - own + reach, start + len - reach))
    };
    let mut candidates: Vec<(f32, f32)> = Vec::new();
    for &(x, y, ow, oh) in &others {
        candidates.push((x, y));
        candidates.push((x + ow, along(raw.1, y, oh, h)));
        candidates.push((x - w, along(raw.1, y, oh, h)));
        candidates.push((along(raw.0, x, ow, w), y + oh));
        candidates.push((along(raw.0, x, ow, w), y - h));
    }
    let free = |c: (f32, f32)| !others.iter().any(|o| overlaps((c.0, c.1, w, h), *o));
    let distance = |c: (f32, f32)| (c.0 - raw.0).hypot(c.1 - raw.1);
    let round = |v: f32| (v * 100.0).round() / 100.0;
    candidates
        .into_iter()
        .filter(|c| free(*c) && distance(*c) < reach)
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .or_else(|| Some((round(raw.0), round(raw.1))).filter(|c| free(*c)))
}

/// A computer in its real proportions: its screens as they stand, the name in the middle.
fn draw_computer(ui: &egui::Ui, rect: egui::Rect, view: &MachineView, screens: Option<&[egui::Rect]>) {
    let p = theme::colors(ui);
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, p.card);
    painter.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    // The same margin all around; number, keyboard and menu sit on plates above the screens
    let inner = rect.shrink(6.0);
    let (frame, fill) = (view.frame(ui), view.screen(ui));
    match screens.and_then(|s| Some((s, s.iter().copied().reduce(|a, b| a.union(b))?))) {
        Some((screens, bounds)) if bounds.width() > 0.0 && bounds.height() > 0.0 => {
            let fit = (inner.width() / bounds.width()).min(inner.height() / bounds.height());
            let size = bounds.size() * fit;
            let origin = inner.center() - size / 2.0;
            for s in screens {
                let r = egui::Rect::from_min_size(origin + (s.min - bounds.min) * fit, s.size() * fit).shrink(1.5);
                painter.rect_filled(r, 4.0, frame);
                painter.rect_filled(r.shrink(2.5), 2.5, fill);
            }
        }
        _ => {
            painter.rect_filled(inner, 4.0, frame);
            painter.rect_filled(inner.shrink(2.5), 2.5, fill);
        }
    }
    // Name and state in the middle, on a plate so they read on any screen
    let line = |text: &str, font: egui::FontId, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width() - 12.0);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let name = line(&view.name, egui::FontId::new(13.5, theme::semibold()), p.text);
    let caption = view.caption().trim();
    let caption = (!caption.is_empty()).then(|| line(caption, egui::FontId::proportional(11.5), p.muted));
    let height = name.size().y + caption.as_ref().map_or(0.0, |c| c.size().y + 1.0);
    let width = name.size().x.max(caption.as_ref().map_or(0.0, |c| c.size().x));
    let plate = egui::Rect::from_center_size(inner.center(), egui::vec2(width + 14.0, height + 8.0));
    painter.rect_filled(plate, 6.0, p.card.gamma_multiply(0.92));
    let top = plate.top() + 4.0;
    painter.galley(
        egui::pos2(rect.center().x - name.size().x / 2.0, top),
        name.clone(),
        p.text,
    );
    if let Some(caption) = caption {
        painter.galley(
            egui::pos2(rect.center().x - caption.size().x / 2.0, top + name.size().y + 1.0),
            caption,
            p.muted,
        );
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
    let plate = egui::Rect::from_min_size(at, egui::vec2(25.5, 9.5)).expand(3.0);
    painter.rect_filled(plate, 4.0, p.card.gamma_multiply(0.92));
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
    let plate = if hovered { p.field } else { p.card.gamma_multiply(0.92) };
    ui.painter().rect_filled(rect, 4.0, plate);
    for i in -1..=1 {
        ui.painter()
            .circle_filled(rect.center() + egui::vec2(i as f32 * 4.5, 0.0), 1.6, p.muted);
    }
}

/// The screens of this computer or of a connected peer, each in its physical size where it is
/// known (so a scaled display setting does not shrink a computer), in points.
fn screens_of(status: &Status, me: &str, id: &str) -> Option<Vec<egui::Rect>> {
    let (screens, sizes) = screens_and_sizes(status, me, id)?;
    let rects: Vec<egui::Rect> = screen_size::footprint(&screens, &sizes)
        .into_iter()
        .map(|[x, y, w, h]| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)))
        .collect();
    (!rects.is_empty()).then_some(rects)
}

/// Screens and their physical sizes in millimeters, in the same order.
type ScreensAndSizes = (Vec<Screen>, Vec<(u32, u32)>);

/// The screens of this computer or of a connected peer and their physical sizes.
fn screens_and_sizes(status: &Status, me: &str, id: &str) -> Option<ScreensAndSizes> {
    if id == me {
        return Some((status.screens.clone(), status.screen_sizes.clone()));
    }
    let peer = status
        .peers
        .iter()
        .find(|peer| layout_file::machine_id(&peer.fingerprint) == id)?;
    Some((peer.link.as_ref()?.screens.clone(), peer.screen_sizes.clone()))
}

#[cfg(test)]
mod tests {
    use super::{Area, shared_edge, snap};

    fn area(m: &str, x: f32, y: f32, w: f32, h: f32) -> Area {
        Area {
            machine: m.into(),
            x,
            y,
            w,
            h,
            measured: true,
        }
    }

    #[test]
    fn dropped_computers_snap_to_edges_and_corners() {
        // A wide iMac and a PC right of it
        let desk = vec![area("imac", 0.0, 0.0, 5.12, 1.44), area("pc", 5.12, 0.0, 1.92, 1.08)];
        let book = (1.728, 1.117);
        // Near the bottom left corner of the iMac: aligned with it
        assert_eq!(snap((0.05, 1.5), book, "book", &desk, 0.2), Some((0.0, 1.44)));
        // Below its middle: centered
        let middle = (5.12 - 1.728) / 2.0;
        assert_eq!(
            snap((middle + 0.1, 1.4), book, "book", &desk, 0.2),
            Some((middle, 1.44))
        );
        // Somewhere else below it: free along the edge
        assert_eq!(snap((0.9, 1.5), book, "book", &desk, 0.2), Some((0.9, 1.44)));
        // Onto the PC: shares its place
        assert_eq!(snap((5.2, 0.05), book, "book", &desk, 0.2), Some((5.12, 0.0)));
        // Over the iMac: not possible
        assert_eq!(snap((1.0, 0.2), book, "book", &desk, 0.2), None);
        // Far away: stays there
        assert_eq!(snap((9.0, 5.0), book, "book", &desk, 0.2), Some((9.0, 5.0)));
    }

    #[test]
    fn green_bars_cover_the_shared_part() {
        let imac = area("imac", 0.0, 0.0, 4.0, 2.0);
        assert_eq!(
            shared_edge(&imac, &area("book", 1.0, 2.0, 2.0, 1.0)),
            Some(((1.0, 2.0), (3.0, 2.0)))
        );
        assert_eq!(
            shared_edge(&imac, &area("pc", 4.0, 1.0, 2.0, 2.0)),
            Some(((4.0, 1.0), (4.0, 2.0)))
        );
        assert_eq!(shared_edge(&imac, &area("far", 5.0, 5.0, 1.0, 1.0)), None);
    }
}
