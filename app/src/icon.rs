//! The TableCross symbol, drawn in code: two screens side by side with an arrow across the gap.

/// RGBA pixels of a square icon. `template`: black on transparent (macOS menu bar), otherwise
/// colored on a rounded tile.
pub fn rgba(size: u32, template: bool) -> Vec<u8> {
    let s = size as f32;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let tile = [0x2B, 0x6C, 0xD9];
    let ink = if template { [0, 0, 0] } else { [0xFF, 0xFF, 0xFF] };
    let accent = if template { [0, 0, 0] } else { [0xFF, 0xB0, 0x2E] };
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut color: Option<[u8; 3]> = None;
            if !template && in_rounded(fx, fy, 0.0, 0.0, s, s, s * 0.22) {
                color = Some(tile);
            }
            // Two screens with a gap between them
            let (top, bottom) = (s * 0.24, s * 0.60);
            let frame = s * 0.06;
            for (left, right) in [(s * 0.07, s * 0.43), (s * 0.57, s * 0.93)] {
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
                // Stand
                let center = (left + right) / 2.0;
                if (fx - center).abs() < s * 0.05 && fy > bottom && fy < bottom + s * 0.09 {
                    color = Some(ink);
                }
                if (fx - center).abs() < s * 0.13 && fy >= bottom + s * 0.09 && fy < bottom + s * 0.13 {
                    color = Some(ink);
                }
            }
            // The pointer travelling from one screen to the other
            let (ay, half) = (s * 0.42, s * 0.045);
            if fx > s * 0.25 && fx < s * 0.62 && (fy - ay).abs() < half {
                color = Some(accent);
            }
            let tip = s * 0.77;
            let dx = tip - fx;
            if dx > 0.0 && dx < s * 0.17 && (fy - ay).abs() < dx * 0.75 {
                color = Some(accent);
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

/// PNG of the colored icon.
pub fn png(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&rgba(size, false)).expect("png data");
    drop(writer);
    out
}

/// Windows icon file with PNG images inside.
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = sizes.iter().map(|s| (*s, png(*s))).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend_from_slice(&data);
    }
    out
}

/// `tablecross --write-icons DIR`: the icon files for packages (PNG sizes and an ICO).
pub fn write_all(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for size in [16, 32, 64, 128, 256, 512, 1024] {
        std::fs::write(dir.join(format!("icon-{size}.png")), png(size))?;
    }
    std::fs::write(dir.join("tablecross.ico"), ico(&[16, 24, 32, 48, 64, 256]))
}
