//! The TableCross symbol, drawn in code: two screens side by side with an arrow across the gap.

/// RGBA pixels of a square icon. `template`: black on transparent (macOS menu bar), otherwise
/// colored on a rounded tile.
pub fn rgba(size: u32, template: bool) -> Vec<u8> {
    let s = size as f32;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let tile = [0x2B, 0x6C, 0xD9];
    let ink = if template { [0, 0, 0] } else { [0xFF, 0xFF, 0xFF] };
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut color: Option<[u8; 3]> = None;
            if !template && in_rounded(fx, fy, 0.0, 0.0, s, s, s * 0.22) {
                color = Some(tile);
            }
            // Two screens
            let (top, bottom) = (s * 0.26, s * 0.62);
            let frame = s * 0.07;
            for (left, right) in [(s * 0.12, s * 0.46), (s * 0.54, s * 0.88)] {
                let outer = in_rounded(fx, fy, left, top, right, bottom, s * 0.05);
                let inner = in_rounded(
                    fx,
                    fy,
                    left + frame,
                    top + frame,
                    right - frame,
                    bottom - frame,
                    s * 0.02,
                );
                if outer && !inner {
                    color = Some(ink);
                }
            }
            // Feet
            for cx in [s * 0.29, s * 0.71] {
                if (fx - cx).abs() < s * 0.06 && fy > bottom && fy < bottom + s * 0.1 {
                    color = Some(ink);
                }
            }
            // Arrow across the gap
            let (ay, half) = (s * 0.44, s * 0.035);
            if fx > s * 0.32 && fx < s * 0.62 && (fy - ay).abs() < half {
                color = Some(ink);
            }
            let tip = s * 0.70;
            let dx = tip - fx;
            if dx > 0.0 && dx < s * 0.14 && (fy - ay).abs() < dx * 0.9 {
                color = Some(ink);
            }
            if let Some(c) = color {
                let i = ((y * size + x) * 4) as usize;
                pixels[i..i + 3].copy_from_slice(&c);
                pixels[i + 3] = 0xFF;
            }
        }
    }
    pixels
}

fn in_rounded(x: f32, y: f32, left: f32, top: f32, right: f32, bottom: f32, radius: f32) -> bool {
    if x < left || x > right || y < top || y > bottom {
        return false;
    }
    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(top + radius, bottom - radius);
    (x - cx).hypot(y - cy) <= radius
}

pub fn window_icon() -> eframe::egui::IconData {
    let size = 256;
    eframe::egui::IconData {
        rgba: rgba(size, false),
        width: size,
        height: size,
    }
}
