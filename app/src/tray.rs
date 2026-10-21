//! Symbol in the menu bar (macOS), notification area (Windows) or status area (Linux, through
//! StatusNotifierItem; GNOME needs the AppIndicator extension) with a small menu. TableCross keeps
//! running when its window is closed; the symbol brings the window back.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, ViewportCommand};
use tc_session::{Command, SessionHandle};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::tr;

pub struct Tray {
    _icon: TrayIcon,
    menu: Menu,
    /// "Update to …", in the menu only while a new version is there.
    update: MenuItem,
    update_shown: std::cell::Cell<bool>,
    show: MenuItem,
    /// "Save files from the clipboard …", active while another computer has files copied.
    save_files: MenuItem,
    release: MenuItem,
    pin: CheckMenuItem,
    lock: MenuItem,
    power_off_others: MenuItem,
    power_off_all: MenuItem,
    quit: MenuItem,
}

/// Linux: whether the desktop shows status icons at all (someone owns the StatusNotifierWatcher
/// name). GNOME does so only with the AppIndicator extension. `None`: cannot tell.
#[cfg(target_os = "linux")]
pub fn icons_shown() -> Option<bool> {
    let out = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.NameHasOwner",
            "org.kde.StatusNotifierWatcher",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let reply = String::from_utf8_lossy(&out.stdout);
    if reply.contains("true") {
        Some(true)
    } else if reply.contains("false") {
        Some(false)
    } else {
        None
    }
}

/// What the menu asked to shut down, kept until the window asked back.
pub const POWER_OFF_OTHERS: u8 = 1;
pub const POWER_OFF_ALL: u8 = 2;

/// After "Quit" the process ends at the latest after this long.
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(4);

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
        session: Arc<Mutex<Option<SessionHandle>>>,
        quit_flag: Arc<AtomicBool>,
        update_flag: Arc<AtomicBool>,
        power_off: Arc<AtomicU8>,
        save_files_flag: Arc<AtomicBool>,
    ) -> Option<Self> {
        let update = MenuItem::new("", true, None);
        let show = MenuItem::new(tr("Show TableCross"), true, None);
        let save_files = MenuItem::new(tr("Save files from the clipboard …"), false, None);
        let release = MenuItem::new(tr("Release everything"), true, None);
        let pin = CheckMenuItem::new(tr("Keep the pointer here"), true, false, None);
        let lock = MenuItem::new(tr("Lock all screens"), true, None);
        let power_off_others = MenuItem::new(tr("Shut down other computers …"), true, None);
        let power_off_all = MenuItem::new(tr("Shut down all computers …"), true, None);
        let quit = MenuItem::new(tr("Quit TableCross"), true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &show,
            &save_files,
            &release,
            &pin,
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
        let pin_id = pin.id().clone();
        let (others_id, all_id) = (power_off_others.id().clone(), power_off_all.id().clone());
        let update_id = update.id().clone();
        let save_files_id = save_files.id().clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                show_window(&menu_ctx);
            } else if event.id == release_id {
                if let Some(session) = session.lock().ok().and_then(|s| s.clone()) {
                    session.send(Command::ReleaseAll);
                }
            } else if event.id == save_files_id {
                save_files_flag.store(true, Ordering::SeqCst);
                show_window(&menu_ctx);
            } else if event.id == update_id {
                update_flag.store(true, Ordering::SeqCst);
                menu_ctx.request_repaint();
            } else if event.id == pin_id {
                if let Some(session) = session.lock().ok().and_then(|s| s.clone()) {
                    session.send(Command::Pin(None));
                }
            } else if event.id == lock_id {
                if let Some(session) = session.lock().ok().and_then(|s| s.clone()) {
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
                tracing::info!("quit chosen in the status icon menu");
                // Stop sharing right away, without waiting for the (maybe hidden) window
                if let Some(session) = session.lock().ok().and_then(|s| s.clone()) {
                    session.send(Command::Shutdown);
                }
                quit_flag.store(true, Ordering::SeqCst);
                menu_ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                menu_ctx.send_viewport_cmd(ViewportCommand::Close);
                menu_ctx.request_repaint();
                // Quit means quit: if the window does not close, end the process anyway
                std::thread::spawn(|| {
                    std::thread::sleep(QUIT_GRACE);
                    tracing::warn!("window did not close after quit, ending the process");
                    std::process::exit(0);
                });
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
            save_files,
            release,
            pin,
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

    /// "Save files from the clipboard …" can be chosen while another computer has files copied.
    pub fn set_files_waiting(&self, waiting: bool) {
        if self.save_files.is_enabled() != waiting {
            self.save_files.set_enabled(waiting);
        }
    }

    /// Ticks "Keep the pointer here" while the pointer is pinned.
    pub fn set_pinned(&self, pinned: bool) {
        if self.pin.is_checked() != pinned {
            self.pin.set_checked(pinned);
        }
    }

    /// After a language change.
    pub fn retranslate(&self) {
        self.show.set_text(tr("Show TableCross"));
        self.save_files.set_text(tr("Save files from the clipboard …"));
        self.release.set_text(tr("Release everything"));
        self.pin.set_text(tr("Keep the pointer here"));
        self.lock.set_text(tr("Lock all screens"));
        self.power_off_others.set_text(tr("Shut down other computers …"));
        self.power_off_all.set_text(tr("Shut down all computers …"));
        self.quit.set_text(tr("Quit TableCross"));
    }
}
