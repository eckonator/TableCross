//! Symbol in the menu bar (macOS), notification area (Windows) or status area (Linux, through
//! StatusNotifierItem; GNOME needs the AppIndicator extension) with a small menu. TableCross keeps
//! running when its window is closed; the symbol brings the window back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use eframe::egui::{self, ViewportCommand};
use tc_session::{Command, SessionHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::tr;

pub struct Tray {
    _icon: TrayIcon,
    menu: Menu,
    /// "Update to …", in the menu only while a new version is there.
    update: MenuItem,
    update_shown: std::cell::Cell<bool>,
    show: MenuItem,
    release: MenuItem,
    lock: MenuItem,
    power_off_others: MenuItem,
    power_off_all: MenuItem,
    quit: MenuItem,
}

/// What the menu asked to shut down, kept until the window asked back.
pub const POWER_OFF_OTHERS: u8 = 1;
pub const POWER_OFF_ALL: u8 = 2;

/// macOS: TableCross lives in the menu bar only, without a Dock icon or an entry in ⌘Tab.
pub fn menu_bar_only() {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// Without a Dock icon, macOS does not bring the window forward by itself.
pub fn bring_to_front() {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        #[allow(deprecated)] // `activate()` needs macOS 14; this works from macOS 11 on
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// Linux: Wayland cannot hide a window at all, and GNOME shows the status icon only with an
/// extension. Minimizing works everywhere and keeps the window reachable from the dock or taskbar.
pub fn hide_window(ctx: &egui::Context) {
    if cfg!(target_os = "linux") {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    } else {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }
}

fn show_window(ctx: &egui::Context) {
    bring_to_front();
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}

impl Tray {
    /// `quit` is set when the user chose "Quit" in the menu, `update_flag` when "Update to …",
    /// `power_off` to `POWER_OFF_OTHERS` or `POWER_OFF_ALL` when the window should ask about that.
    pub fn new(
        ctx: &egui::Context,
        session: Option<SessionHandle>,
        quit_flag: Arc<AtomicBool>,
        update_flag: Arc<AtomicBool>,
        power_off: Arc<AtomicU8>,
    ) -> Option<Self> {
        let update = MenuItem::new("", true, None);
        let show = MenuItem::new(tr("Show TableCross"), true, None);
        let release = MenuItem::new(tr("Release everything"), true, None);
        let lock = MenuItem::new(tr("Lock all screens"), true, None);
        let power_off_others = MenuItem::new(tr("Shut down other computers …"), true, None);
        let power_off_all = MenuItem::new(tr("Shut down all computers …"), true, None);
        let quit = MenuItem::new(tr("Quit TableCross"), true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &show,
            &release,
            &lock,
            &power_off_others,
            &power_off_all,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .ok()?;
        let template = cfg!(target_os = "macos");
        let size = if template { 36 } else { 32 };
        let icon = Icon::from_rgba(crate::icon::rgba(size, template), size, size).ok()?;
        // macOS draws the menu bar icon from its shape only, in the menu bar's color
        #[cfg(target_os = "macos")]
        let builder = TrayIconBuilder::new().with_icon_templated(icon);
        #[cfg(not(target_os = "macos"))]
        let builder = TrayIconBuilder::new().with_icon(icon);
        let tray = builder
            .with_tooltip("TableCross")
            .with_menu(Box::new(menu.clone()))
            .build();
        let tray = match tray {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("no status icon: {e}");
                return None;
            }
        };
        let (show_id, release_id, quit_id) = (show.id().clone(), release.id().clone(), quit.id().clone());
        let lock_id = lock.id().clone();
        let (others_id, all_id) = (power_off_others.id().clone(), power_off_all.id().clone());
        let update_id = update.id().clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                show_window(&menu_ctx);
            } else if event.id == release_id {
                if let Some(session) = &session {
                    session.send(Command::ReleaseAll);
                }
            } else if event.id == update_id {
                update_flag.store(true, Ordering::SeqCst);
                menu_ctx.request_repaint();
            } else if event.id == lock_id {
                if let Some(session) = &session {
                    session.send(Command::LockAll);
                }
            } else if event.id == others_id || event.id == all_id {
                let which = if event.id == all_id {
                    POWER_OFF_ALL
                } else {
                    POWER_OFF_OTHERS
                };
                power_off.store(which, Ordering::SeqCst);
                show_window(&menu_ctx);
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
            menu,
            update,
            update_shown: std::cell::Cell::new(false),
            show,
            release,
            lock,
            power_off_others,
            power_off_all,
            quit,
        })
    }

    /// Shows "Update to `version`" at the top of the menu, or removes it (`None`).
    pub fn set_update(&self, version: Option<&str>) {
        match version {
            Some(version) => {
                self.update.set_text(crate::i18n::trf("Update to {}", &[&version]));
                if !self.update_shown.replace(true) {
                    let _ = self.menu.insert(&self.update, 0);
                }
            }
            None => {
                if self.update_shown.replace(false) {
                    let _ = self.menu.remove(&self.update);
                }
            }
        }
    }

    /// After a language change.
    pub fn retranslate(&self) {
        self.show.set_text(tr("Show TableCross"));
        self.release.set_text(tr("Release everything"));
        self.lock.set_text(tr("Lock all screens"));
        self.power_off_others.set_text(tr("Shut down other computers …"));
        self.power_off_all.set_text(tr("Shut down all computers …"));
        self.quit.set_text(tr("Quit TableCross"));
    }
}
