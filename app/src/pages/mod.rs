//! The pages of the window.

mod about;
pub mod arrangement;
mod pairing;
mod settings_page;
mod setup;
mod status;

use eframe::egui;

pub use crate::theme::card;

pub fn ok_color(ui: &egui::Ui) -> egui::Color32 {
    crate::theme::colors(ui).ok
}

pub fn warn_color(ui: &egui::Ui) -> egui::Color32 {
    crate::theme::colors(ui).warn
}

/// A link that opens in the browser (egui's own links need a feature that is not built in).
pub fn link(ui: &mut egui::Ui, text: &str, url: &str) {
    if ui.link(text).on_hover_text(url).clicked() {
        open(url);
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
