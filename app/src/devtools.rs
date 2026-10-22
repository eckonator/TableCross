//! Development helpers, read from the environment:
//!
//! - `TC_SCREENSHOT=/path/shot.bmp` saves a picture of the window shortly after start and quits.
//! - `TC_DEMO=1` shows made-up computers instead of starting the network and input (nothing is saved).
//! - `TC_PAGE=status|arrangement|pairing|settings|about` opens that page; `TC_SETUP=1|2|3` the setup step.
//! - `TC_LANG=de|en` and `TC_THEME=light|dark` override the settings without saving them.
//! - `TC_SIZE=900x640` sets the window size.

use std::io::Write;
use std::path::{Path, PathBuf};

use eframe::egui::{self, ColorImage, Event, ViewportCommand};

const SCREENSHOT_AFTER_FRAMES: u64 = 20;

#[derive(Default)]
pub struct DevTools {
    screenshot: Option<PathBuf>,
    pub demo: bool,
    pub page: Option<String>,
    /// Tab of the settings page (`general`, `pointer`, `mac`, `network`, `troubleshooting`).
    pub tab: Option<String>,
    pub setup_step: Option<usize>,
    pub lang: Option<String>,
    pub theme: Option<String>,
    frame: u64,
    requested: bool,
}

impl DevTools {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Self {
            screenshot: var("TC_SCREENSHOT").map(PathBuf::from),
            demo: var("TC_DEMO").is_some(),
            page: var("TC_PAGE"),
            tab: var("TC_TAB"),
            setup_step: var("TC_SETUP").and_then(|s| s.parse().ok()),
            lang: var("TC_LANG"),
            theme: var("TC_THEME"),
            frame: 0,
            requested: false,
        }
    }

    /// Developer runs never save anything.
    pub fn isolated(&self) -> bool {
        self.screenshot.is_some() || self.demo || self.lang.is_some() || self.theme.is_some()
    }

    pub fn window_size() -> Option<[f32; 2]> {
        let size = std::env::var("TC_SIZE").ok()?;
        let (w, h) = size.split_once('x')?;
        Some([w.parse().ok()?, h.parse().ok()?])
    }

    /// Called every frame.
    pub fn frame(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        self.frame += 1;
        if !self.requested && self.frame >= SCREENSHOT_AFTER_FRAMES {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        ctx.request_repaint();
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            if let Err(e) = save_bmp(&path, &image) {
                tracing::error!("screenshot not saved: {e}");
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    pub fn quitting_after_screenshot(&self) -> bool {
        self.screenshot.is_some()
    }
}

/// Uncompressed 24-bit BMP.
fn save_bmp(path: &Path, img: &ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"BM")?;
    f.write_all(&(size as u32).to_le_bytes())?;
    f.write_all(&[0; 4])?;
    f.write_all(&54u32.to_le_bytes())?;
    f.write_all(&40u32.to_le_bytes())?;
    f.write_all(&(w as i32).to_le_bytes())?;
    f.write_all(&(h as i32).to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&24u16.to_le_bytes())?;
    f.write_all(&[0; 24])?;
    let padding = vec![0u8; row - w * 3];
    for y in (0..h).rev() {
        for x in 0..w {
            let c = img.pixels[y * w + x];
            f.write_all(&[c.b(), c.g(), c.r()])?;
        }
        f.write_all(&padding)?;
    }
    f.flush()
}
