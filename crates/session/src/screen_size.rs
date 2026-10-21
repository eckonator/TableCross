//! Physical size of screens: what they report, checked for plausibility, and how large they stand
//! on the desk. Each screen counts in its own size, so a laptop next to an external display shows
//! both as they are, whatever their display setting.

use serde::{Deserialize, Serialize};
use tc_protocol::Screen;

/// Points per millimeter a screen of known physical size counts on the desk: about what macOS
/// shows by default (110 points per inch), so sizes from both sources fit together.
pub const POINTS_PER_MM: f32 = 110.0 / 25.4;
/// Points that touch closer than this belong to one edge.
const TOUCH: f32 = 2.0;

/// A rectangle in points: x, y, width, height.
pub type Rect = [f32; 4];

/// The logical size of a screen in points (`None` for an empty one).
fn points(s: &Screen) -> Option<(f32, f32)> {
    (s.width > 0 && s.height > 0 && s.scale > 0.0).then(|| (s.width as f32 / s.scale, s.height as f32 / s.scale))
}

/// The size in millimeters the right way round for the screen (displays report it unrotated).
fn turned(s: &Screen, (mm_w, mm_h): (u32, u32)) -> Option<(f32, f32)> {
    let (w, h) = points(s)?;
    if mm_w == 0 || mm_h == 0 {
        return None;
    }
    Some(if (w > h) == (mm_w >= mm_h) {
        (mm_w as f32, mm_h as f32)
    } else {
        (mm_h as f32, mm_w as f32)
    })
}

/// Whether `mm` fits this screen: the same shape, and 60 to 350 points per inch. Some displays
/// report a made-up size (a 28-inch one claimed 63 inches), virtual machines anything.
pub fn plausible(s: &Screen, mm: (u32, u32)) -> bool {
    let (Some((w, h)), Some((mm_w, mm_h))) = (points(s), turned(s, mm)) else {
        return false;
    };
    let shape = (w / h) / (mm_w / mm_h);
    let per_inch = w / (mm_w / 25.4);
    (0.85..=1.15).contains(&shape) && (60.0..=350.0).contains(&per_inch)
}

/// How much larger the screen stands on the desk than its points say (1 without a plausible size).
pub fn factor(s: &Screen, mm: Option<(u32, u32)>) -> f32 {
    match (
        points(s),
        mm.filter(|mm| plausible(s, *mm)).and_then(|mm| turned(s, mm)),
    ) {
        (Some((w, _)), Some((mm_w, _))) => mm_w * POINTS_PER_MM / w,
        _ => 1.0,
    }
}

/// The screens in their physical size, in points, with the top left corner of all at 0, 0. Each
/// screen keeps the edges it shares with the others, as in the display settings.
pub fn footprint(screens: &[Screen], mm: &[(u32, u32)]) -> Vec<Rect> {
    let logical: Vec<Option<Rect>> = screens
        .iter()
        .map(|s| points(s).map(|(w, h)| [s.x as f32 / s.scale, s.y as f32 / s.scale, w, h]))
        .collect();
    let factors: Vec<f32> = screens
        .iter()
        .enumerate()
        .map(|(i, s)| factor(s, mm.get(i).copied()))
        .collect();
    let mut placed: Vec<Option<Rect>> = vec![None; screens.len()];
    let Some(first) = logical.iter().position(Option::is_some) else {
        return Vec::new();
    };
    let size = |i: usize| {
        let r = logical[i].expect("a screen with points");
        (r[2] * factors[i], r[3] * factors[i])
    };
    let (w, h) = size(first);
    placed[first] = Some([0.0, 0.0, w, h]);
    loop {
        let mut progress = false;
        for j in 0..screens.len() {
            let Some(rj) = logical[j].filter(|_| placed[j].is_none()) else {
                continue;
            };
            // Next to a screen already placed, along the same edge
            let next = (0..screens.len()).find_map(|i| {
                let (ri, pi) = (logical[i]?, placed[i]?);
                let f = factors[i];
                let (w, h) = size(j);
                let along_y = |rj: Rect| pi[1] + along(rj[1] - ri[1], (ri[3], rj[3]), (pi[3], h), f);
                let along_x = |rj: Rect| pi[0] + along(rj[0] - ri[0], (ri[2], rj[2]), (pi[2], w), f);
                let overlaps_y = rj[1] < ri[1] + ri[3] && ri[1] < rj[1] + rj[3];
                let overlaps_x = rj[0] < ri[0] + ri[2] && ri[0] < rj[0] + rj[2];
                if overlaps_y && (rj[0] - (ri[0] + ri[2])).abs() < TOUCH {
                    Some([pi[0] + pi[2], along_y(rj), w, h])
                } else if overlaps_y && (rj[0] + rj[2] - ri[0]).abs() < TOUCH {
                    Some([pi[0] - w, along_y(rj), w, h])
                } else if overlaps_x && (rj[1] - (ri[1] + ri[3])).abs() < TOUCH {
                    Some([along_x(rj), pi[1] + pi[3], w, h])
                } else if overlaps_x && (rj[1] + rj[3] - ri[1]).abs() < TOUCH {
                    Some([along_x(rj), pi[1] - h, w, h])
                } else {
                    None
                }
            });
            if let Some(r) = next {
                placed[j] = Some(r);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    // Screens that touch none of the others keep their distance from the first one
    let r0 = logical[first].expect("the first screen");
    for j in 0..screens.len() {
        if let (Some(rj), None) = (logical[j], placed[j]) {
            let (w, h) = size(j);
            let f = factors[first];
            placed[j] = Some([(rj[0] - r0[0]) * f, (rj[1] - r0[1]) * f, w, h]);
        }
    }
    let rects: Vec<Rect> = placed.into_iter().flatten().collect();
    let left = rects.iter().map(|r| r[0]).fold(f32::INFINITY, f32::min);
    let top = rects.iter().map(|r| r[1]).fold(f32::INFINITY, f32::min);
    rects
        .into_iter()
        .map(|r| [r[0] - left, r[1] - top, r[2], r[3]])
        .collect()
}

/// Where a screen starts along the edge of its neighbor, from where it starts in points: lying
/// within the neighbor's length (or the neighbor within its own), the share of the room to move
/// is kept, so flush at the top or bottom stays flush; else the points count in the neighbor's
/// size (`factor`).
fn along(offset: f32, (len, len_next): (f32, f32), (size, size_next): (f32, f32), factor: f32) -> f32 {
    let room = len - len_next;
    let share = offset / room;
    if room.abs() >= 1.0 && (-1e-3..=1.0 + 1e-3).contains(&share) {
        share * (size - size_next)
    } else {
        offset * factor
    }
}

/// Width and height of all screens together on the desk, in thousands of points (`None` without screens).
pub fn desk_size(screens: &[Screen], mm: &[(u32, u32)]) -> Option<(f32, f32)> {
    let rects = footprint(screens, mm);
    let w = rects.iter().map(|r| r[0] + r[2]).fold(0.0f32, f32::max);
    let h = rects.iter().map(|r| r[1] + r[3]).fold(0.0f32, f32::max);
    (!rects.is_empty()).then_some((w / 1000.0, h / 1000.0))
}

/// A diagonal typed in for the screens that report about this size (one model reports its own
/// size whatever its display setting, give or take a few percent).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Typed {
    /// What the screen reports, in millimeters.
    pub reported: (u32, u32),
    pub inches: f32,
}

impl Typed {
    /// Whether this is meant for a screen reporting `mm`.
    pub fn fits(&self, mm: (u32, u32)) -> bool {
        let near = |a: u32, b: u32| (a as f32 - b as f32).abs() <= 0.03 * b.max(1) as f32;
        near(mm.0, self.reported.0) && near(mm.1, self.reported.1)
    }
}

/// The diagonal in inches of a size in millimeters.
pub fn inches((mm_w, mm_h): (u32, u32)) -> f32 {
    (mm_w as f32).hypot(mm_h as f32) / 25.4
}

/// The sizes this computer tells the others: the diagonal typed in for a screen; else what it
/// reports where that is plausible; else the size of a screen with the same points elsewhere on
/// the desk (often the same model); else what it reports.
pub fn effective(
    screens: &[Screen],
    reported: &[(u32, u32)],
    typed: &[Typed],
    others: &[(Screen, (u32, u32))],
) -> Vec<(u32, u32)> {
    if reported.len() != screens.len() {
        return reported.to_vec();
    }
    screens
        .iter()
        .zip(reported)
        .map(|(s, &mm)| {
            let Some((w, h)) = points(s) else { return mm };
            if let Some(t) = typed.iter().find(|t| t.fits(mm) && (5.0..=150.0).contains(&t.inches)) {
                let per_point = t.inches * 25.4 / w.hypot(h);
                return ((w * per_point).round() as u32, (h * per_point).round() as u32);
            }
            if plausible(s, mm) {
                return mm;
            }
            others
                .iter()
                .find(|(o, o_mm)| {
                    points(o).is_some_and(|(ow, oh)| (ow - w).abs() < 1.0 && (oh - h).abs() < 1.0)
                        && plausible(o, *o_mm)
                })
                .and_then(|(o, o_mm)| turned(o, *o_mm))
                .map_or(mm, |(mm_w, mm_h)| (mm_w.round() as u32, mm_h.round() as u32))
        })
        .collect()
}

/// Whether one of the screens reports a size that cannot be right.
pub fn any_implausible(screens: &[Screen], reported: &[(u32, u32)]) -> bool {
    screens.iter().zip(reported).any(|(s, mm)| !plausible(s, *mm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: i32, y: i32, width: u32, height: u32, scale: f32) -> Screen {
        Screen {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    #[test]
    fn scaled_displays_count_in_their_physical_size() {
        // A 27-inch display looking like 2560 × 1440 by default and like 2048 × 1152 when scaled:
        // both come out the same size on the desk
        let default = desk_size(&[screen(0, 0, 2560, 1440, 1.0)], &[(597, 336)]).unwrap();
        let scaled = desk_size(&[screen(0, 0, 2048, 1152, 1.0)], &[(597, 336)]).unwrap();
        assert!((default.0 - 2.586).abs() < 0.01, "{default:?}");
        assert!((default.0 - scaled.0).abs() < 0.001);
        // Windows at 150 %: logical points count
        let windows = desk_size(&[screen(0, 0, 3840, 2160, 1.5)], &[(597, 336)]).unwrap();
        assert!((default.0 - windows.0).abs() < 0.001);
        // A display turned upright reports its size lying
        assert!(plausible(&screen(0, 0, 1440, 2560, 1.0), (597, 336)));
    }

    #[test]
    fn implausible_sizes_are_ignored() {
        let full_hd = screen(0, 0, 1920, 1080, 1.0);
        assert!(!plausible(&full_hd, (0, 0)));
        // Virtual machines: made-up sizes that do not fit the shape or the density
        assert!(!plausible(&full_hd, (100, 100)));
        assert!(!plausible(&full_hd, (2000, 1125)));
        // A 28-inch display at 1920 × 1080 claiming 63 inches (measured on a real desk)
        assert!(!plausible(&full_hd, (1393, 784)));
        assert_eq!(desk_size(&[full_hd], &[(1393, 784)]), Some((1.92, 1.08)));
        // Lists out of step
        assert_eq!(desk_size(&[full_hd], &[]), Some((1.92, 1.08)));
    }

    #[test]
    fn each_screen_counts_in_its_own_size() {
        // A 16-inch laptop with a 28-inch 4K display to its right, looking like 1920 × 1080
        let screens = [screen(0, 0, 1728, 1117, 1.0), screen(1728, 0, 1920, 1080, 1.0)];
        let rects = footprint(&screens, &[(344, 223), (621, 341)]);
        let (laptop, display) = (rects[0], rects[1]);
        assert!((laptop[2] - 1490.0).abs() < 2.0, "{laptop:?}");
        assert!((display[2] - 2689.0).abs() < 2.0, "{display:?}");
        // Still side by side, tops in line
        assert!((display[0] - laptop[2]).abs() < 0.01);
        assert!((display[1] - laptop[1]).abs() < 0.01);
        // The display is taller: the laptop is drawn from the top, nothing above 0
        assert_eq!(rects.iter().map(|r| r[1]).fold(f32::INFINITY, f32::min), 0.0);
    }

    #[test]
    fn a_laptop_flush_with_the_bottom_of_a_display_stays_flush() {
        // As arranged in the display settings: the laptop to the right, bottoms in line
        let screens = [screen(0, 0, 2560, 1440, 1.0), screen(2560, 323, 1728, 1117, 1.0)];
        let rects = footprint(&screens, &[(619, 340), (344, 223)]);
        let bottom = |r: Rect| r[1] + r[3];
        assert!((bottom(rects[0]) - bottom(rects[1])).abs() < 0.01, "{rects:?}");
        // Flush at the top stays so as well, halfway stays halfway
        let rects = footprint(
            &[screens[0], screen(2560, 0, 1728, 1117, 1.0)],
            &[(619, 340), (344, 223)],
        );
        assert!((rects[0][1] - rects[1][1]).abs() < 0.01, "{rects:?}");
        let rects = footprint(
            &[screens[0], screen(2560, 161, 1728, 1117, 1.0)],
            &[(619, 340), (344, 223)],
        );
        let middle = |r: Rect| r[1] + r[3] / 2.0;
        assert!((middle(rects[0]) - middle(rects[1])).abs() < 1.0, "{rects:?}");
    }

    #[test]
    fn a_display_left_of_the_main_one_ends_where_the_main_one_starts() {
        let screens = [screen(0, 0, 2560, 1440, 1.0), screen(-2560, 0, 2560, 1440, 1.0)];
        let rects = footprint(&screens, &[(619, 340), (619, 340)]);
        assert!(
            (rects[1][0]).abs() < 0.01 && (rects[1][2] - rects[0][0]).abs() < 0.01,
            "{rects:?}"
        );
    }

    #[test]
    fn a_display_with_a_made_up_size_borrows_or_takes_the_diagonal() {
        let lenovo = screen(0, 0, 1920, 1080, 1.0);
        let on_the_other_desk = [(screen(1728, 0, 1920, 1080, 1.0), (621, 341))];
        // The same display elsewhere tells the size
        assert_eq!(
            effective(&[lenovo], &[(1393, 784)], &[], &on_the_other_desk),
            [(621, 341)]
        );
        // A diagonal typed in goes first: 28 inches at 16:9, also when the display setting
        // changed what the screen reports a little
        let typed = [Typed {
            reported: (1383, 778),
            inches: 28.0,
        }];
        assert_eq!(
            effective(&[lenovo], &[(1393, 784)], &typed, &on_the_other_desk),
            [(620, 349)]
        );
        // Nothing to go by: as reported (counts in points)
        assert_eq!(effective(&[lenovo], &[(1393, 784)], &[], &[]), [(1393, 784)]);
        // A diagonal typed in corrects a plausible size as well, but only for that screen
        let laptop = screen(1920, 0, 1728, 1117, 1.0);
        let corrected = effective(
            &[lenovo, laptop],
            &[(621, 341), (344, 223)],
            &[Typed {
                reported: (620, 340),
                inches: 32.0,
            }],
            &[],
        );
        assert_eq!(corrected[1], (344, 223));
        assert!((inches(corrected[0]) - 32.0).abs() < 0.1, "{corrected:?}");
        assert!(any_implausible(&[lenovo], &[(1393, 784)]));
    }
}
