//! Symbol in the menu bar (macOS) or notification area (Windows) with a small menu. TableCross keeps
//! running when its window is closed; the symbol brings the window back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, ViewportCommand};
use tc_session::{Command, SessionHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::tr;

pub struct Tray {
    _icon: TrayIcon,
    show: MenuItem,
    release: MenuItem,
    quit: MenuItem,
}

fn show_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}

impl Tray {
    /// `quit` is set when the user chose "Quit" in the menu.
    pub fn new(ctx: &egui::Context, session: Option<SessionHandle>, quit_flag: Arc<AtomicBool>) -> Option<Self> {
        let show = MenuItem::new(tr("Show TableCross"), true, None);
        let release = MenuItem::new(tr("Release everything"), true, None);
        let quit = MenuItem::new(tr("Quit TableCross"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&show, &release, &PredefinedMenuItem::separator(), &quit])
            .ok()?;
        let template = cfg!(target_os = "macos");
        let size = if template { 36 } else { 32 };
        let icon = Icon::from_rgba(crate::icon::rgba(size, template), size, size).ok()?;
        let builder = TrayIconBuilder::new();
        let builder = if template {
            builder.with_icon_templated(icon)
        } else {
            builder.with_icon(icon)
        };
        let tray = builder.with_tooltip("TableCross").with_menu(Box::new(menu)).build();
        let tray = match tray {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("no status icon: {e}");
                return None;
            }
        };
        let (show_id, release_id, quit_id) = (show.id().clone(), release.id().clone(), quit.id().clone());
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                show_window(&menu_ctx);
            } else if event.id == release_id {
                if let Some(session) = &session {
                    session.send(Command::ReleaseAll);
                }
            } else if event.id == quit_id {
                quit_flag.store(true, Ordering::SeqCst);
                menu_ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                menu_ctx.send_viewport_cmd(ViewportCommand::Close);
                menu_ctx.request_repaint();
            }
        }));
        // Windows: a click on the symbol opens the window, the menu is on the right button
        if cfg!(windows) {
            let click_ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_window(&click_ctx);
                }
            }));
        }
        Some(Self {
            _icon: tray,
            show,
            release,
            quit,
        })
    }

    /// After a language change.
    pub fn retranslate(&self) {
        self.show.set_text(tr("Show TableCross"));
        self.release.set_text(tr("Release everything"));
        self.quit.set_text(tr("Quit TableCross"));
    }
}
