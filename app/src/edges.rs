//! Green bars at the real screen edges where the pointer crosses, while the arrangement is being
//! changed: on this computer from the arrangement page, on the others through `ShowEdges`. Each bar
//! is a small borderless window that stays on top and lets clicks through. Wayland does not let a
//! program place windows, so there the bars do not appear.

use eframe::egui;
use tc_core::Area;
use tc_protocol::{EdgeMark, Screen, Side};

/// Thickness of a bar in points.
const THICKNESS: f32 = 8.0;
/// How many bars at most (a few edges per computer).
const MAX_BARS: usize = 16;

/// Where the pointer crosses on each machine of `areas`: parts of its sides that touch another
/// machine, from 0.0 to 1.0 along the side. Every machine is listed, with no marks if it touches
/// nothing (so its bars go away).
pub fn marks(areas: &[Area]) -> Vec<(String, Vec<EdgeMark>)> {
    areas
        .iter()
        .map(|a| {
            let marks = areas
                .iter()
                .filter(|b| b.machine != a.machine && (b.x != a.x || b.y != a.y))
                .filter_map(|b| {
                    let side = tc_core::area_side(a, b)?;
                    let (start, len, other, other_len) = match side {
                        Side::Left | Side::Right => (a.y, a.h, b.y, b.h),
                        Side::Top | Side::Bottom => (a.x, a.w, b.x, b.w),
                    };
                    let from = ((other.max(start) - start) / len).clamp(0.0, 1.0);
                    let to = (((other + other_len).min(start + len) - start) / len).clamp(0.0, 1.0);
                    (to > from).then_some(EdgeMark { side, from, to })
                })
                .collect();
            (a.machine.clone(), marks)
        })
        .collect()
}

/// The bars for `marks` on `screens`, in points of the desktop.
fn bars(screens: &[Screen], marks: &[EdgeMark]) -> Vec<egui::Rect> {
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
    let Some(bounds) = rects.iter().copied().reduce(|a, b| a.union(b)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for mark in marks {
        let vertical = matches!(mark.side, Side::Left | Side::Right);
        let range = if vertical {
            egui::Rangef::new(
                bounds.top() + mark.from * bounds.height(),
                bounds.top() + mark.to * bounds.height(),
            )
        } else {
            egui::Rangef::new(
                bounds.left() + mark.from * bounds.width(),
                bounds.left() + mark.to * bounds.width(),
            )
        };
        // Only screens at the outer side of the computer have that edge
        for r in &rects {
            let (outer, span) = match mark.side {
                Side::Left => ((r.left() - bounds.left()).abs() < 1.0, r.y_range()),
                Side::Right => ((r.right() - bounds.right()).abs() < 1.0, r.y_range()),
                Side::Top => ((r.top() - bounds.top()).abs() < 1.0, r.x_range()),
                Side::Bottom => ((r.bottom() - bounds.bottom()).abs() < 1.0, r.x_range()),
            };
            let (min, max) = (range.min.max(span.min), range.max.min(span.max));
            if !outer || max - min < 1.0 {
                continue;
            }
            out.push(match mark.side {
                Side::Left => egui::Rect::from_x_y_ranges(r.left()..=r.left() + THICKNESS, min..=max),
                Side::Right => egui::Rect::from_x_y_ranges(r.right() - THICKNESS..=r.right(), min..=max),
                Side::Top => egui::Rect::from_x_y_ranges(min..=max, r.top()..=r.top() + THICKNESS),
                Side::Bottom => egui::Rect::from_x_y_ranges(min..=max, r.bottom() - THICKNESS..=r.bottom()),
            });
        }
    }
    out.truncate(MAX_BARS);
    out
}

/// Shows the bars for `marks` on this computer's `screens` (call every frame while they should be
/// seen; they disappear in the first frame without the call).
pub fn show(ctx: &egui::Context, screens: &[Screen], marks: &[EdgeMark], color: egui::Color32) {
    for (i, bar) in bars(screens, marks).into_iter().enumerate() {
        let builder = egui::ViewportBuilder::default()
            .with_title("TableCross")
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_active(false)
            .with_position(bar.min)
            .with_inner_size(bar.size());
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("tc-edge-bar", i)), builder, |ui, _| {
            ui.painter().rect_filled(ui.ctx().content_rect(), 0.0, color);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn marks_cover_the_shared_part_of_each_side() {
        // A laptop below the left half of a wide iMac
        let areas = [area("imac", 0.0, 0.0, 4.0, 2.0), area("book", 0.0, 2.0, 2.0, 1.0)];
        let marks = marks(&areas);
        assert_eq!(
            marks[0],
            (
                "imac".to_string(),
                vec![EdgeMark {
                    side: Side::Bottom,
                    from: 0.0,
                    to: 0.5
                }]
            )
        );
        assert_eq!(
            marks[1].1,
            [EdgeMark {
                side: Side::Top,
                from: 0.0,
                to: 1.0
            }]
        );
    }

    #[test]
    fn bars_sit_on_the_outer_screens() {
        // Two screens side by side, the bar on the left half of the bottom edge: only the left one
        let screen = |x: i32| Screen {
            x,
            y: 0,
            width: 1000,
            height: 500,
            scale: 1.0,
        };
        let mark = EdgeMark {
            side: Side::Bottom,
            from: 0.0,
            to: 0.5,
        };
        let bars = bars(&[screen(0), screen(1000)], &[mark]);
        assert_eq!(bars, [egui::Rect::from_x_y_ranges(0.0..=1000.0, 492.0..=500.0)]);
        // The right side of two screens side by side is only on the right one
        let mark = EdgeMark {
            side: Side::Right,
            from: 0.0,
            to: 1.0,
        };
        let bars = super::bars(&[screen(0), screen(1000)], &[mark]);
        assert_eq!(bars, [egui::Rect::from_x_y_ranges(1992.0..=2000.0, 0.0..=500.0)]);
    }
}
