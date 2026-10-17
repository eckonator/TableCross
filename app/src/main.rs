//! TableCross: share mouse, keyboard and clipboard between the computers on your desk.

// No extra console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod demo;
mod devtools;
mod edges;
mod i18n;
mod icon;
mod logging;
mod pages;
mod self_update;
mod settings;
mod texts;
mod theme;
mod tray;
mod uninstall;
mod update;

use eframe::egui;

use crate::devtools::DevTools;
use crate::settings::{LangSetting, Settings};

fn main() -> eframe::Result {
    // For building packages: write the icon files and quit
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--write-icons") {
        let dir = args
            .get(2)
            .map_or_else(|| std::path::PathBuf::from("assets"), std::path::PathBuf::from);
        if let Err(e) = icon::write_all(&dir) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let dev = DevTools::from_env();
    let dir = tc_net::default_config_dir().unwrap_or_else(|| std::env::temp_dir().join("TableCross"));
    let log_file = if dev.isolated() {
        tracing_subscriber::fmt().with_env_filter("info").init();
        None
    } else {
        logging::init(&dir)
    };
    tracing::info!("TableCross {} starting", env!("CARGO_PKG_VERSION"));
    // The demo shows made-up values, not this computer's settings
    let mut settings = if dev.demo {
        Settings {
            name: "MacBook".into(),
            update_check: Some(true),
            setup_done: true,
            ..Settings::default()
        }
    } else {
        Settings::load(&dir)
    };
    if let Some(lang) = &dev.lang {
        settings.language = if lang == "de" { LangSetting::De } else { LangSetting::En };
    }
    i18n::apply_setting(settings.language);
    let background = std::env::args().any(|a| a == autostart::BACKGROUND_FLAG);

    let size = DevTools::window_size().unwrap_or([860.0, 640.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TableCross")
            .with_app_id("tablecross")
            .with_inner_size(size)
            .with_min_inner_size([720.0, 520.0])
            .with_icon(icon::window_icon()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "TableCross",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, dir, settings, dev, background, log_file)))),
    )
}
