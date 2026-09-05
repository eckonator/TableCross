//! The pages of the window.

mod about;
mod computers;
mod settings_page;
mod setup;
mod status;

use eframe::egui::{self, RichText};

/// A framed section with a title.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .inner_margin(12.0)
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            ui.add_space(6.0);
            add(ui)
        })
        .inner
}

pub fn ok_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0x5C, 0xC8, 0x6E)
    } else {
        egui::Color32::from_rgb(0x1E, 0x8E, 0x3E)
    }
}

pub fn warn_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0xF0, 0xB4, 0x4C)
    } else {
        egui::Color32::from_rgb(0xB0, 0x60, 0x00)
    }
}

/// Opens a URL or file with the system.
pub fn open(target: &str) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(target).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("explorer").arg(target).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(target).spawn()
    };
    if let Err(e) = result {
        tracing::warn!("cannot open {target}: {e}");
    }
}

#[cfg(target_os = "macos")]
pub const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// A small round state light (the font has no reliable circle glyph).
pub fn dot(ui: &mut egui::Ui, color: egui::Color32, filled: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    if filled {
        ui.painter().circle_filled(rect.center(), 4.5, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), 4.0, egui::Stroke::new(1.5, color));
    }
}
