//! Points, rectangles and the screens of one machine, in that machine's native units.

use tc_protocol::Screen;
pub use tc_protocol::Side;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Axis-aligned rectangle; contains `[left, right) × [top, bottom)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    /// Native units per logical point.
    pub scale: f64,
}

impl Rect {
    pub fn from_screen(s: &Screen) -> Self {
        let scale = if s.scale.is_finite() && s.scale > 0.0 {
            f64::from(s.scale)
        } else {
            1.0
        };
        Self {
            left: f64::from(s.x),
            top: f64::from(s.y),
            right: f64::from(s.x) + f64::from(s.width),
            bottom: f64::from(s.y) + f64::from(s.height),
            scale,
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }

    /// The nearest point inside.
    pub fn clamp(&self, p: Point) -> Point {
        Point::new(
            p.x.clamp(self.left, (self.right - 1.0).max(self.left)),
            p.y.clamp(self.top, (self.bottom - 1.0).max(self.top)),
        )
    }

    fn distance(&self, p: Point) -> f64 {
        let c = self.clamp(p);
        (c.x - p.x).hypot(c.y - p.y)
    }
}

/// All screens of one machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Screens {
    rects: Vec<Rect>,
}

impl Screens {
    /// Screens without area are ignored. `None` if nothing usable is left.
    pub fn new(screens: &[Screen]) -> Option<Self> {
        let rects: Vec<Rect> = screens
            .iter()
            .filter(|s| s.width > 0 && s.height > 0)
            .map(Rect::from_screen)
            .collect();
        (!rects.is_empty()).then_some(Self { rects })
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn contains(&self, p: Point) -> bool {
        self.rects.iter().any(|r| r.contains(p))
    }

    /// The screen containing `p`, or else the nearest one.
    pub fn screen_at(&self, p: Point) -> &Rect {
        self.rects
            .iter()
            .find(|r| r.contains(p))
            .or_else(|| self.rects.iter().min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))))
            .expect("at least one screen")
    }

    pub fn bounds(&self) -> Rect {
        let mut b = self.rects[0];
        for r in &self.rects[1..] {
            b.left = b.left.min(r.left);
            b.top = b.top.min(r.top);
            b.right = b.right.max(r.right);
            b.bottom = b.bottom.max(r.bottom);
        }
        b
    }

    /// Where the step from `from` (on a screen) to `to` leaves this machine, if it does.
    pub fn exit_side(&self, from: Point, to: Point) -> Option<Side> {
        if self.contains(to) {
            return None;
        }
        let s = self.screen_at(from);
        let over_x = if to.x >= s.right {
            Some((Side::Right, to.x - s.right + 1.0))
        } else if to.x < s.left {
            Some((Side::Left, s.left - to.x))
        } else {
            None
        };
        let over_y = if to.y >= s.bottom {
            Some((Side::Bottom, to.y - s.bottom + 1.0))
        } else if to.y < s.top {
            Some((Side::Top, s.top - to.y))
        } else {
            None
        };
        match (over_x, over_y) {
            (Some((x, dx)), Some((y, dy))) => Some(if dx >= dy { x } else { y }),
            (Some((side, _)), None) | (None, Some((side, _))) => Some(side),
            (None, None) => None,
        }
    }

    /// Position along `side` of the whole machine, 0.0 to 1.0.
    pub fn along(&self, side: Side, p: Point) -> f64 {
        let b = self.bounds();
        let a = if side.is_horizontal() {
            (p.y - b.top) / (b.bottom - b.top)
        } else {
            (p.x - b.left) / (b.right - b.left)
        };
        a.clamp(0.0, 1.0)
    }

    /// Distance of `p` from the nearer corner of its screen's `side`, in logical points.
    pub fn corner_distance(&self, side: Side, p: Point) -> f64 {
        let s = self.screen_at(p);
        let d = if side.is_horizontal() {
            (p.y - s.top).min(s.bottom - 1.0 - p.y)
        } else {
            (p.x - s.left).min(s.right - 1.0 - p.x)
        };
        d.max(0.0) / s.scale
    }

    /// Distance of `p` from `side` of its screen, in logical points.
    pub fn edge_distance(&self, side: Side, p: Point) -> f64 {
        let s = self.screen_at(p);
        let d = match side {
            Side::Left => p.x - s.left,
            Side::Right => s.right - 1.0 - p.x,
            Side::Top => p.y - s.top,
            Side::Bottom => s.bottom - 1.0 - p.y,
        };
        d.max(0.0) / s.scale
    }

    /// Where the pointer appears when it comes in through `side` at `along` (0.0 to 1.0).
    pub fn entry(&self, side: Side, along: f64) -> Point {
        let b = self.bounds();
        let along = along.clamp(0.0, 1.0);
        // Coordinate along the edge, then the outermost screen there
        let (pos, covers): (f64, fn(&Rect, f64) -> bool) = if side.is_horizontal() {
            (b.top + along * (b.bottom - b.top), |r, v| v >= r.top && v < r.bottom)
        } else {
            (b.left + along * (b.right - b.left), |r, v| v >= r.left && v < r.right)
        };
        let outermost = |r: &&Rect| match side {
            Side::Left => -r.left,
            Side::Right => r.right,
            Side::Top => -r.top,
            Side::Bottom => r.bottom,
        };
        let screen = self
            .rects
            .iter()
            .filter(|r| covers(r, pos))
            .max_by(|a, b| outermost(a).total_cmp(&outermost(b)))
            .or_else(|| self.rects.iter().max_by(|a, b| outermost(a).total_cmp(&outermost(b))))
            .expect("at least one screen");
        let p = match side {
            Side::Left => Point::new(screen.left, pos),
            Side::Right => Point::new(screen.right - 1.0, pos),
            Side::Top => Point::new(pos, screen.top),
            Side::Bottom => Point::new(pos, screen.bottom - 1.0),
        };
        screen.clamp(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn screen(x: i32, y: i32, w: u32, h: u32, scale: f32) -> Screen {
        Screen {
            x,
            y,
            width: w,
            height: h,
            scale,
        }
    }

    #[test]
    fn exit_only_where_no_other_screen_is() {
        // Two screens side by side, the right one lower
        let s = Screens::new(&[screen(0, 0, 100, 100, 1.0), screen(100, 50, 100, 100, 1.0)]).unwrap();
        let p = Point::new(99.0, 60.0);
        assert_eq!(s.exit_side(p, Point::new(101.0, 60.0)), None);
        // Upper part of the right edge of the left screen is open
        let p = Point::new(99.0, 10.0);
        assert_eq!(s.exit_side(p, Point::new(100.0, 10.0)), Some(Side::Right));
        assert_eq!(
            s.exit_side(Point::new(0.0, 0.0), Point::new(-1.0, 0.0)),
            Some(Side::Left)
        );
        assert_eq!(
            s.exit_side(Point::new(150.0, 149.0), Point::new(150.0, 151.0)),
            Some(Side::Bottom)
        );
        // Diagonal: the larger overshoot decides
        assert_eq!(
            s.exit_side(Point::new(0.0, 0.0), Point::new(-5.0, -1.0)),
            Some(Side::Left)
        );
    }

    #[test]
    fn entry_on_the_outermost_screen() {
        let s = Screens::new(&[screen(0, 0, 100, 100, 1.0), screen(100, 50, 100, 100, 1.0)]).unwrap();
        // Coming in from the right at the top: only the left screen covers y = 0
        let p = s.entry(Side::Right, 0.0);
        assert_eq!(p, Point::new(99.0, 0.0));
        // Lower part: the right screen is further out
        let p = s.entry(Side::Right, 0.9);
        assert_eq!(p, Point::new(199.0, 135.0));
        assert_eq!(s.entry(Side::Left, 0.5), Point::new(0.0, 75.0));
        assert_eq!(s.entry(Side::Top, 0.75), Point::new(150.0, 50.0));
        assert_eq!(s.entry(Side::Bottom, 1.0), Point::new(199.0, 149.0));
    }

    #[test]
    fn along_and_corners() {
        let s = Screens::new(&[screen(0, 0, 200, 100, 2.0)]).unwrap();
        assert_eq!(s.along(Side::Right, Point::new(199.0, 25.0)), 0.25);
        assert_eq!(s.along(Side::Top, Point::new(50.0, 0.0)), 0.25);
        // 10 native units at scale 2 are 5 logical points
        assert_eq!(s.corner_distance(Side::Right, Point::new(199.0, 10.0)), 5.0);
        assert_eq!(s.corner_distance(Side::Right, Point::new(199.0, 99.0)), 0.0);
    }

    #[test]
    fn unusable_screens() {
        assert!(Screens::new(&[]).is_none());
        assert!(Screens::new(&[screen(0, 0, 0, 100, 1.0)]).is_none());
        let s = Screens::new(&[screen(0, 0, 10, 10, f32::NAN)]).unwrap();
        assert_eq!(s.rects()[0].scale, 1.0);
    }
}
