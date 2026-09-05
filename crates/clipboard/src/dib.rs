//! Windows bitmaps (`CF_DIB`: BITMAPINFOHEADER and pixels) ↔ PNG. Plain code, tested everywhere.

const HEADER: usize = 40;
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
/// Larger images are not converted (memory).
const MAX_PIXELS: usize = 64 * 1024 * 1024;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn i32_at(b: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// 24- and 32-bit DIBs (what screenshots and image programs put there). `None` for anything else.
pub fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let header_size = u32_at(dib, 0)? as usize;
    let width = i32_at(dib, 4)?;
    let height = i32_at(dib, 8)?;
    let bits = u16_at(dib, 14)?;
    let compression = u32_at(dib, 16)?;
    let colors_used = u32_at(dib, 32)? as usize;
    if header_size < HEADER || width <= 0 || height == 0 || !(bits == 24 || bits == 32) {
        return None;
    }
    if compression != BI_RGB && !(compression == BI_BITFIELDS && bits == 32) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if w.checked_mul(h)? > MAX_PIXELS {
        return None;
    }
    // Bit field masks follow a plain BITMAPINFOHEADER; we assume the usual BGRA order
    let masks = if compression == BI_BITFIELDS && header_size == HEADER {
        12
    } else {
        0
    };
    let offset = header_size + masks + colors_used * 4;
    let bytes_pp = usize::from(bits / 8);
    let stride = (w * bytes_pp).div_ceil(4) * 4;
    let pixels = dib.get(offset..offset.checked_add(stride.checked_mul(h)?)?)?;
    // Alpha only counts if anything is not fully transparent (many programs leave it at zero)
    let alpha_used = bits == 32 && (0..h).any(|y| (0..w).any(|x| pixels[y * stride + x * 4 + 3] != 0));
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        // Positive height: bottom-up
        let y = if height > 0 { h - 1 - row } else { row };
        let line = &pixels[y * stride..];
        for x in 0..w {
            let p = &line[x * bytes_pp..];
            let alpha = if alpha_used { p[3] } else { 0xFF };
            rgba.extend_from_slice(&[p[2], p[1], p[0], alpha]);
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

/// A 32-bit top-down DIB for programs that do not read PNG from the clipboard.
pub fn png_to_dib(png_bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let (w, h) = (reader.info().width as usize, reader.info().height as usize);
    if w.checked_mul(h)? > MAX_PIXELS {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..frame.buffer_size()];
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return None,
    };
    let mut dib = Vec::with_capacity(HEADER + w * h * 4);
    dib.extend_from_slice(&(HEADER as u32).to_le_bytes());
    dib.extend_from_slice(&(w as i32).to_le_bytes());
    dib.extend_from_slice(&(-(h as i32)).to_le_bytes()); // top-down
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&BI_RGB.to_le_bytes());
    dib.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    dib.extend_from_slice(&[0; 16]);
    for i in 0..w * h {
        let p = &data[i * channels..];
        let (r, g, b, a) = match channels {
            4 => (p[0], p[1], p[2], p[3]),
            3 => (p[0], p[1], p[2], 0xFF),
            2 => (p[0], p[0], p[0], p[1]),
            _ => (p[0], p[0], p[0], 0xFF),
        };
        dib.extend_from_slice(&[b, g, r, a]);
    }
    Some(dib)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        let pixel = &pixel;
        let data: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| pixel(x, y))).collect();
        writer.write_image_data(&data).unwrap();
        drop(writer);
        out
    }

    fn pixels(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf[..info.buffer_size()].to_vec())
    }

    #[test]
    fn png_dib_png_keeps_the_pixels() {
        let original = png_of(5, 3, |x, y| {
            [x as u8 * 40, y as u8 * 80, 7, if x == 0 { 128 } else { 255 }]
        });
        let dib = png_to_dib(&original).unwrap();
        let back = dib_to_png(&dib).unwrap();
        assert_eq!(pixels(&back), pixels(&original));
    }

    #[test]
    fn bottom_up_24_bit_with_padding() {
        // 3×2, 24 bit, rows padded to 12 bytes, bottom row first
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&3i32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        let bottom = [[0, 0, 255], [0, 255, 0], [255, 0, 0]]; // BGR: red, green, blue
        let top = [[255, 255, 255]; 3];
        for row in [bottom, top] {
            for p in row {
                dib.extend_from_slice(&p);
            }
            dib.extend_from_slice(&[0; 3]);
        }
        let (w, h, rgba) = pixels(&dib_to_png(&dib).unwrap());
        assert_eq!((w, h), (3, 2));
        assert_eq!(&rgba[..4], &[255, 255, 255, 255], "top-left is white");
        assert_eq!(&rgba[12..16], &[255, 0, 0, 255], "bottom-left is red");
        assert_eq!(&rgba[20..24], &[0, 0, 255, 255], "bottom-right is blue");
    }

    #[test]
    fn garbage_is_refused() {
        assert!(dib_to_png(&[]).is_none());
        assert!(dib_to_png(&[40, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x7F]).is_none());
        let mut huge = vec![0u8; 40];
        huge[0] = 40;
        huge[4..8].copy_from_slice(&100_000i32.to_le_bytes());
        huge[8..12].copy_from_slice(&100_000i32.to_le_bytes());
        huge[14] = 32;
        assert!(dib_to_png(&huge).is_none());
        assert!(png_to_dib(b"not a png").is_none());
    }
}
