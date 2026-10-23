//! The window: top bar, navigation, pages. Pages only draw and send commands to the session.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, ViewportCommand};
use tc_net::Fingerprint;
use tc_session::{Command, SessionConfig, SessionHandle, Status};

use crate::devtools::DevTools;
use crate::i18n::{self, tr};
use crate::settings::{Settings, ThemeSetting};
use crate::theme;

/// How long bars sent by another computer stay after its last message.
const EDGES_SHOWN: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Status,
    Arrangement,
    Pairing,
    Settings,
    About,
}

/// Background work that blocks (system dialogs), with its result.
pub type Job = std::sync::mpsc::Receiver<Result<(), String>>;

pub struct App {
    pub dir: PathBuf,
    pub settings: Settings,
    /// Settings as they were when the session started (name, port, discovery need a restart).
    pub started_with: Settings,
    pub session: Option<SessionHandle>,
    runtime: Option<tokio::runtime::Runtime>,
    task: Option<tokio::task::JoinHandle<()>>,
    pub status: Status,
    pub demo: Option<Status>,
    pub page: Page,
    /// Step of the setup (0, 1, 2) while it is shown.
    pub setup_step: Option<usize>,
    pub code_input: String,
    pub address_input: String,
    /// "Add an address" on the pairing page: for which computer, and what was typed.
    pub peer_address: Option<(tc_net::Fingerprint, String)>,
    pub pair_target: Option<Fingerprint>,
    pub confirm_unpair: Option<Fingerprint>,
    pub info: Option<(String, Instant)>,
    pub job: Option<Job>,
    pub isolated: bool,
    pub log_file: Option<PathBuf>,
    quit_flag: Arc<AtomicBool>,
    /// "Update to …" was chosen in the menu of the status icon.
    update_flag: Arc<AtomicBool>,
    /// "Shut down …" was chosen in the menu of the status icon (`tray::POWER_OFF_*`).
    power_off_flag: Arc<AtomicU8>,
    /// Asking whether to shut down (or log out with `.0`); `.1` includes this computer.
    confirm_power_off: Option<(bool, bool)>,
    /// "Save files from the clipboard …" was chosen in the menu of the status icon.
    save_files_flag: Arc<AtomicBool>,
    /// Asking where the files copied on another computer go: the folder chosen so far.
    save_files_to: Option<PathBuf>,
    /// Linux: the desktop shows no status icons (GNOME without the AppIndicator extension), and
    /// when that was last looked at.
    pub icons_missing: bool,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    icons_checked: Option<Instant>,
    tray: Option<crate::tray::Tray>,
    dev: DevTools,
    background: bool,
    logo: Option<egui::TextureHandle>,
    /// A newer version found on GitHub.
    pub new_version: Option<crate::update::NewVersion>,
    update_job: Option<std::sync::mpsc::Receiver<Result<Option<crate::update::NewVersion>, crate::update::CheckError>>>,
    /// The running check was asked for by hand: say what came out.
    update_manual: bool,
    /// A paired computer runs this newer version: install it as soon as the check finds it.
    follow_peer: Option<String>,
    /// Newer peer versions already followed (once each).
    followed: Vec<String>,
    update_checked: Option<Instant>,
    /// Installing a new version from inside the app, and how far it got.
    self_update: Option<std::sync::mpsc::Receiver<crate::self_update::Progress>>,
    pub update_progress: Option<crate::self_update::Progress>,
    /// After the system helper was replaced: until then, show where to grant its permission as
    /// soon as it reports that it lacks it (a replaced helper may lose it).
    pub helper_prompt: Option<Instant>,
    /// Removing TableCross: settings page choices, and the removal once started.
    pub remove_settings: bool,
    pub confirm_remove: bool,
    removal: Option<Removal>,
    /// The session the status icon menu talks to (it changes when the session restarts).
    tray_session: Arc<std::sync::Mutex<Option<SessionHandle>>>,
    /// Started anew (or tried to) for a permission granted meanwhile.
    #[cfg(target_os = "macos")]
    restart_tried: bool,
    /// Keeps the system (and with `true` the display) awake while the settings ask for it.
    stay_awake: Option<(bool, tc_input::system::KeepAwake)>,
}

/// How often to look for a new version while running.
const UPDATE_EVERY: Duration = Duration::from_secs(3600);
struct Removal {
    remove_settings: bool,
    started: Instant,
    job: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}

/// How long after replacing the system helper its missing permission opens the settings by itself.
pub const HELPER_PROMPT_FOR: Duration = Duration::from_secs(90);

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        dir: PathBuf,
        settings: Settings,
        dev: DevTools,
        background: bool,
        log_file: Option<PathBuf>,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install_fonts(&ctx);
        theme::apply(&ctx);
        let logo = Some(ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([128, 128], &crate::icon::rgba(128, false)),
            egui::TextureOptions::LINEAR,
        ));
        let isolated = dev.isolated();
        let demo = dev.demo.then(crate::demo::status);
        let mut app = Self {
            dir,
            started_with: settings.clone(),
            settings,
            session: None,
            runtime: None,
            task: None,
            status: Status::default(),
            demo,
            page: Page::Status,
            setup_step: None,
            code_input: String::new(),
            address_input: String::new(),
            peer_address: None,
            pair_target: None,
            confirm_unpair: None,
            info: None,
            job: None,
            isolated,
            log_file,
            quit_flag: Arc::new(AtomicBool::new(false)),
            update_flag: Arc::new(AtomicBool::new(false)),
            power_off_flag: Arc::new(AtomicU8::new(0)),
            confirm_power_off: None,
            save_files_flag: Arc::new(AtomicBool::new(false)),
            save_files_to: None,
            icons_missing: false,
            icons_checked: None,
            tray: None,
            dev,
            background,
            logo,
            new_version: None,
            update_job: None,
            update_manual: false,
            follow_peer: None,
            followed: Vec::new(),
            update_checked: None,
            self_update: None,
            remove_settings: true,
            confirm_remove: false,
            removal: None,
            helper_prompt: std::env::args()
                .any(|a| a == crate::self_update::AFTER_UPDATE_FLAG)
                .then(|| Instant::now() + HELPER_PROMPT_FOR),
            update_progress: None,
            stay_awake: None,
            tray_session: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(target_os = "macos")]
            restart_tried: false,
        };
        app.apply_stay_awake();
        if app.demo.is_none() {
            app.start_session(&ctx);
        }
        app.page = match app.dev.page.as_deref() {
            Some("arrangement" | "computers") => Page::Arrangement,
            Some("pairing") => Page::Pairing,
            Some("settings") => Page::Settings,
            Some("about") => Page::About,
            _ => Page::Status,
        };
        if let Some(tab) = app.dev.tab.as_deref() {
            app.settings.settings_tab =
                serde_json::from_value(serde_json::Value::String(tab.into())).unwrap_or_default();
        }
        let first_start = !app.settings.setup_done && app.current_status().peers.is_empty();
        app.setup_step = app
            .dev
            .setup_step
            .map(|s| s.clamp(1, 3) - 1)
            .or((first_start && app.dev.page.is_none()).then_some(0));
        if !app.dev.quitting_after_screenshot() {
            app.tray = crate::tray::Tray::new(
                &ctx,
                app.tray_session.clone(),
                app.quit_flag.clone(),
                app.update_flag.clone(),
                app.power_off_flag.clone(),
                app.save_files_flag.clone(),
            );
            // Only with a status icon: otherwise the window could not be found again
            if app.tray.is_some() {
                crate::tray::menu_bar_only();
                if !background {
                    crate::tray::bring_to_front();
                }
            }
        }
        if background && app.has_tray() {
            crate::tray::hide_window(&ctx);
        }
        app
    }

    fn has_tray(&self) -> bool {
        self.tray.is_some()
    }

    fn start_session(&mut self, ctx: &egui::Context) {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("tablecross-net")
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("cannot start the runtime: {e}");
                return;
            }
        };
        #[cfg(target_os = "macos")]
        if self.settings.universal_control_off && !self.isolated && tc_input::universal_control::is_on() {
            match tc_input::universal_control::turn_off() {
                Ok(()) => tracing::info!("Universal Control of macOS was on, switched it off"),
                Err(e) => tracing::warn!("cannot switch off Universal Control: {e}"),
            }
        }
        #[cfg(target_os = "macos")]
        if self.settings.handoff_off && !self.isolated && tc_input::handoff::is_on() {
            match tc_input::handoff::turn_off() {
                Ok(()) => tracing::info!("Handoff of macOS was on, switched it off"),
                Err(e) => tracing::warn!("cannot switch off Handoff: {e}"),
            }
        }
        let backend = match tc_input::platform() {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::warn!("no input backend: {e}");
                None
            }
        };
        let config = SessionConfig {
            dir: self.dir.clone(),
            name: self.settings.effective_name(),
            port: self.settings.port,
            discovery: self.settings.discovery,
        };
        let repaint = ctx.clone();
        let on_change: tc_session::OnChange = Arc::new(move || repaint.request_repaint());
        let clipboard = self
            .settings
            .clipboard
            .then(tc_clipboard::platform)
            .flatten()
            .map(|system| tc_session::Clipboard {
                system: system.into(),
                max_size: self.settings.clipboard_max_mb as usize * 1024 * 1024,
            });
        match runtime.block_on(tc_session::Session::start(config, backend, clipboard, Some(on_change))) {
            Ok((handle, task)) => {
                if self.settings.collect_logs {
                    handle.send(Command::CollectLogs(true));
                }
                handle.send(Command::EdgeRule(self.settings.edges.rule()));
                handle.send(Command::Corner(f64::from(self.settings.corner)));
                handle.send(Command::SkipOffline(self.settings.skip_offline));
                handle.send(Command::QuietRadio(self.settings.quiet_radio));
                handle.send(Command::FollowLayout(self.settings.follow_layout));
                if cfg!(target_os = "macos") && !self.isolated {
                    handle.send(Command::LoginScreen(self.settings.login_screen));
                }
                handle.send(Command::ScreenDiagonals(self.settings.screen_diagonals.clone()));
                *self.tray_session.lock().expect("tray session lock") = Some(handle.clone());
                self.session = Some(handle);
                self.task = Some(task);
            }
            Err(e) => {
                tracing::error!("cannot start: {e}");
                self.info = Some((format!("{}: {e}", tr("TableCross could not start")), Instant::now()));
            }
        }
        self.runtime = Some(runtime);
        self.started_with = self.settings.clone();
    }

    /// Ends the app for good. A hidden window may never get to close (it is not drawn), and a new
    /// version waits for this process to end: so the session stops right away, the window shows
    /// to close, and if it still does not, the process ends after a grace period.
    fn quit(&mut self, ctx: &egui::Context, why: &'static str) {
        self.quit_flag.store(true, Ordering::SeqCst);
        self.stop_session();
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Close);
        ctx.request_repaint();
        std::thread::spawn(move || {
            std::thread::sleep(crate::tray::QUIT_GRACE);
            tracing::warn!("window did not close ({why}), ending the process");
            std::process::exit(0);
        });
    }

    fn stop_session(&mut self) {
        *self.tray_session.lock().expect("tray session lock") = None;
        if let Some(session) = self.session.take() {
            session.send(Command::Shutdown);
        }
        if let (Some(runtime), Some(task)) = (self.runtime.take(), self.task.take()) {
            let _ = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(3), task).await });
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }

    /// Stops and starts the session with the current settings (name, port, discovery).
    pub fn restart_session(&mut self, ctx: &egui::Context) {
        self.stop_session();
        self.start_session(ctx);
    }

    pub fn needs_restart(&self) -> bool {
        self.demo.is_none()
            && (self.settings.effective_name() != self.started_with.effective_name()
                || self.settings.port != self.started_with.port
                || self.settings.discovery != self.started_with.discovery
                || self.settings.clipboard != self.started_with.clipboard
                || self.settings.clipboard_max_mb != self.started_with.clipboard_max_mb)
    }

    pub fn current_status(&self) -> Status {
        match (&self.demo, &self.session) {
            (Some(demo), _) => demo.clone(),
            (None, Some(session)) => session.status(),
            (None, None) => Status::default(),
        }
    }

    pub fn send(&self, command: Command) {
        if let Some(session) = &self.session {
            session.send(command);
        }
    }

    pub fn save_settings(&self) {
        if !self.isolated {
            self.settings.save(&self.dir);
        }
    }

    /// Takes or lets go of the wake lock after the settings.
    pub fn apply_stay_awake(&mut self) {
        let wanted = (self.settings.stay_awake && !self.isolated).then_some(self.settings.screen_stays_on);
        if wanted == self.stay_awake.as_ref().map(|(screen, _)| *screen) {
            return;
        }
        self.stay_awake = None;
        match wanted {
            Some(screen) => {
                let lock = if screen {
                    tc_input::system::KeepAwake::new()
                } else {
                    tc_input::system::KeepAwake::system_only()
                };
                match lock {
                    Some(lock) => {
                        tracing::info!("keeping this computer from going to sleep (screen on: {screen})");
                        self.stay_awake = Some((screen, lock));
                    }
                    None => tracing::warn!("cannot keep this computer from going to sleep"),
                }
            }
            None => tracing::info!("this computer may go to sleep again"),
        }
    }

    pub fn show_info(&mut self, text: impl Into<String>) {
        self.info = Some((text.into(), Instant::now()));
    }

    /// Runs `work` on a thread of its own (system dialogs block); the result shows up as info.
    #[cfg(target_os = "macos")]
    pub fn run_job(&mut self, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        self.job = Some(rx);
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let theme = match self.dev.theme.as_deref() {
            Some("dark") => ThemeSetting::Dark,
            Some("light") => ThemeSetting::Light,
            _ => self.settings.theme,
        };
        ctx.set_theme(match theme {
            ThemeSetting::System => egui::ThemePreference::System,
            ThemeSetting::Light => egui::ThemePreference::Light,
            ThemeSetting::Dark => egui::ThemePreference::Dark,
        });
    }

    pub fn language_changed(&self) {
        i18n::apply_setting(self.settings.language);
        if let Some(tray) = &self.tray {
            tray.retranslate();
        }
    }

    /// Left column: logo, state, pages, emergency button.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = theme::colors(ui);
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(34.0, 34.0)));
            }
            ui.label(
                RichText::new("TableCross")
                    .font(egui::FontId::new(19.0, theme::semibold()))
                    .color(p.text),
            );
        });
        ui.add_space(14.0);
        let (text, fg, bg) = match (&self.status.controlling, &self.status.controlled_by) {
            (Some(name), _) => (i18n::trf("Controlling {}", &[name]), p.on_accent, p.accent),
            (_, Some(name)) => (i18n::trf("Controlled by {}", &[name]), p.text, p.warn_soft),
            _ => (tr("Working on this computer").to_string(), p.muted, p.window),
        };
        // Long computer names end in "…", else the column grows wider than it is shown
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let room = ui.available_width() - 6.0 - 18.0;
            egui::Frame::new()
                .fill(bg)
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(9, 3))
                .show(ui, |ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(RichText::new(&text).size(12.0).color(fg)).truncate())
                        .on_hover_text(&text);
                });
        });
        if self.status.pinned {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let chip = egui::Button::new(RichText::new(tr("Pointer kept here ✕")).size(12.0).color(p.on_accent))
                    .fill(p.accent)
                    .corner_radius(10.0);
                if ui
                    .add(chip)
                    .on_hover_text(tr(
                        "The pointer does not cross edges (Ctrl+Shift+Alt+F). Click to let it go.",
                    ))
                    .clicked()
                {
                    self.send(Command::Pin(Some(false)));
                }
            });
        }
        ui.add_space(18.0);
        // The status symbol shows how things are: orange with a problem, green when connected
        let health =
            if !self.status.problems.is_empty() || matches!(self.status.input, tc_session::InputState::Failed(_)) {
                p.warn
            } else if self.status.peers.iter().any(|peer| peer.link.is_some()) {
                p.ok
            } else {
                p.muted
            };
        for (page, symbol, label) in [
            (Page::Status, "◉", tr("Status")),
            (Page::Arrangement, "▣", tr("Arrangement")),
            (Page::Pairing, "⇄", tr("Pairing")),
            (Page::Settings, "⚙", tr("Settings")),
            (Page::About, "ℹ", tr("About")),
        ] {
            let selected = self.page == page && self.setup_step.is_none();
            let color = (page == Page::Status).then_some(health);
            if nav_item(ui, symbol, label, selected, color).clicked() {
                self.page = page;
                self.setup_step = None;
            }
        }
        ui.add_space(6.0);
        if nav_item(ui, "✦", tr("Setup"), self.setup_step.is_some(), None).clicked() {
            self.setup_step = Some(0);
        }
        let mut install = false;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .small()
                        .color(p.muted),
                );
            });
            // A new version: one click installs it and starts it again
            if let Some(new) = self.new_version.clone().filter(|n| n.asset.is_some()) {
                ui.add_space(6.0);
                if self
                    .update_progress
                    .as_ref()
                    .is_some_and(|p| !matches!(p, crate::self_update::Progress::Failed(_)))
                {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label(RichText::new(tr("Updating …")).small().color(p.muted));
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        install = theme::primary_button(ui, &i18n::trf("Update to {}", &[&new.version])).clicked();
                    });
                }
            }
            ui.add_space(6.0);
            let release = ui
                .add_sized(
                    [ui.available_width() - 12.0, 34.0],
                    egui::Button::new(RichText::new(tr("Release everything")).color(p.text)),
                )
                .on_hover_text(tr(
                    "All keys up, mouse and keyboard back to this computer (Ctrl+Shift+Alt+Esc)",
                ));
            if release.clicked() {
                self.send(Command::ReleaseAll);
            }
        });
        if install {
            self.install_update(ui.ctx());
        }
    }

    /// Title and one line below it at the top of a page.
    pub fn page_header(&self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        let p = theme::colors(ui);
        ui.add_space(6.0);
        ui.label(RichText::new(title).heading().color(p.text));
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).color(p.muted));
        }
        ui.add_space(14.0);
    }

    /// Starts the update check when allowed and due, and picks up its result.
    fn update_check(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.update_job
            && let Ok(result) = job.try_recv()
        {
            self.update_job = None;
            let manual = std::mem::take(&mut self.update_manual);
            match result {
                Ok(new) => {
                    if manual && new.is_none() {
                        self.show_info(i18n::trf("TableCross {} is up to date.", &[&env!("CARGO_PKG_VERSION")]));
                    }
                    self.new_version = new;
                    // Started because a paired computer is newer: install right away
                    if let (Some(target), Some(found)) = (self.follow_peer.take(), &self.new_version)
                        && found.asset.is_some()
                        && !crate::update::newer(&target, &found.version)
                        && self.update_progress.is_none()
                    {
                        tracing::info!("updating to {} like a paired computer", found.version);
                        self.install_update(ctx);
                    }
                }
                Err(crate::update::CheckError::NoTool) if manual => self.show_info(tr(
                    "Neither curl nor wget is installed, which the update check needs (Linux: sudo apt install curl).",
                )),
                Err(_) if manual => self.show_info(tr("GitHub could not be reached; try again later.")),
                Err(_) => {}
            }
        }
        self.follow_peers(ctx);
        self.check_icons();
        if let Some(tray) = &self.tray {
            let version = self
                .new_version
                .as_ref()
                .filter(|n| n.asset.is_some())
                .map(|n| n.version.as_str());
            tray.set_update(version);
            tray.set_pinned(self.status.pinned);
            tray.set_files_waiting(self.status.files_waiting.as_ref().is_some_and(|w| !w.pasteable));
        }
        if self.update_flag.swap(false, Ordering::SeqCst) {
            self.install_update(ctx);
        }
        let progress: Vec<_> = self.self_update.iter().flat_map(|job| job.try_iter()).collect();
        // At `Restart` the update has ended this version already
        if let Some(progress) = progress.into_iter().last() {
            self.update_progress = Some(progress);
        }
        let due = self.update_checked.is_none_or(|t| t.elapsed() >= UPDATE_EVERY);
        if self.settings.update_check == Some(true) && self.demo.is_none() && self.update_job.is_none() && due {
            self.update_checked = Some(Instant::now());
            self.update_job = Some(crate::update::check_in_background(ctx.clone()));
        }
    }

    /// A paired computer runs a newer version: look for it now (and install it when found).
    fn follow_peers(&mut self, ctx: &egui::Context) {
        if !self.settings.update_with_peers || self.demo.is_some() || self.update_job.is_some() {
            return;
        }
        let newest = self
            .status
            .peers
            .iter()
            .filter_map(|p| p.link.as_ref().map(|l| l.app_version.clone()))
            .filter(|v| crate::update::newer(v, env!("CARGO_PKG_VERSION")))
            .max_by(|a, b| {
                if crate::update::newer(a, b) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                }
            });
        let Some(version) = newest else { return };
        if self.followed.contains(&version) {
            return;
        }
        tracing::info!("a paired computer runs {version}, looking for it");
        self.followed.push(version.clone());
        self.follow_peer = Some(version);
        self.update_checked = Some(Instant::now());
        self.update_job = Some(crate::update::check_in_background(ctx.clone()));
    }

    /// Looks for a new version now (button), also if regular checks are off.
    pub fn check_updates_now(&mut self, ctx: &egui::Context) {
        if self.update_job.is_some() || self.demo.is_some() {
            return;
        }
        self.update_manual = true;
        self.update_checked = Some(Instant::now());
        self.update_job = Some(crate::update::check_in_background(ctx.clone()));
    }

    pub fn checking_updates(&self) -> bool {
        self.update_job.is_some()
    }

    pub fn install_update(&mut self, ctx: &egui::Context) {
        // One at a time: a second run would share the first one's files (button and following a
        // paired computer at once)
        if self
            .update_progress
            .as_ref()
            .is_some_and(|p| !matches!(p, crate::self_update::Progress::Failed(_)))
        {
            tracing::info!("an update is already running");
            return;
        }
        if let Some(url) = self.new_version.as_ref().and_then(|n| n.asset.clone()) {
            self.update_progress = Some(crate::self_update::Progress::Downloading);
            let (session, quit_flag, quit_ctx) = (self.tray_session.clone(), self.quit_flag.clone(), ctx.clone());
            self.self_update = Some(crate::self_update::start(url, ctx.clone(), move || {
                crate::tray::quit_now(&session, &quit_flag, &quit_ctx, "update")
            }));
        }
    }

    /// Removes TableCross: first the paired computers forget this one, then the session stops and
    /// everything is removed in the background; the app quits when done.
    pub fn start_removal(&mut self) {
        if self.remove_settings {
            for peer in &self.status.peers {
                self.send(Command::Unpair(peer.fingerprint));
            }
        }
        self.removal = Some(Removal {
            remove_settings: self.remove_settings,
            started: Instant::now(),
            job: None,
        });
    }

    fn poll_removal(&mut self, ctx: &egui::Context) {
        let Some(removal) = &mut self.removal else { return };
        ctx.request_repaint_after(Duration::from_millis(200));
        if removal.job.is_none() {
            // Time for the unpair messages to go out
            if removal.started.elapsed() < Duration::from_millis(1500) {
                return;
            }
            let remove_settings = removal.remove_settings;
            let dir = self.dir.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::uninstall::run(&dir, remove_settings));
            });
            if let Some(removal) = &mut self.removal {
                removal.job = Some(rx);
            }
            self.stop_session();
            return;
        }
        let result = removal.job.as_ref().and_then(|job| job.try_recv().ok());
        match result {
            Some(Ok(())) => {
                tracing::info!("TableCross removed");
                self.quit(ctx, "removal");
            }
            Some(Err(e)) => {
                self.removal = None;
                self.confirm_remove = false;
                self.show_info(i18n::trf("That did not work: {}", &[&e]));
                self.start_session(ctx);
            }
            None => {}
        }
    }

    pub fn removing(&self) -> bool {
        self.removal.is_some()
    }

    /// Asks before shutting down what the menu of the status icon chose.
    fn power_off_dialog(&mut self, ctx: &egui::Context) {
        match self.power_off_flag.swap(0, Ordering::SeqCst) {
            crate::tray::POWER_OFF_OTHERS => self.confirm_power_off = Some((false, false)),
            crate::tray::POWER_OFF_ALL => self.confirm_power_off = Some((false, true)),
            crate::tray::LOG_OUT_OTHERS => self.confirm_power_off = Some((true, false)),
            crate::tray::LOG_OUT_ALL => self.confirm_power_off = Some((true, true)),
            _ => {}
        }
        let Some((log_out, this_one)) = self.confirm_power_off else {
            return;
        };
        let others: Vec<String> = self
            .status
            .peers
            .iter()
            .filter(|p| p.link.is_some())
            .map(|p| p.name.clone())
            .collect();
        if !this_one && others.is_empty() {
            self.confirm_power_off = None;
            self.show_info(tr("No other computer is connected."));
            return;
        }
        let p = theme::palette(ctx);
        let mut decided = None;
        let modal = egui::Modal::new(egui::Id::new("power_off")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            let title = match (log_out, this_one) {
                (false, true) => tr("Shut down all computers?"),
                (false, false) => tr("Shut down the other computers?"),
                (true, true) => tr("Log out on all computers?"),
                (true, false) => tr("Log out on the other computers?"),
            };
            ui.label(RichText::new(title).size(16.0).strong());
            ui.add_space(6.0);
            let list = if others.is_empty() {
                tr("Only this computer is connected.").to_string()
            } else if this_one {
                i18n::trf("This computer and {}", &[&others.join(", ")])
            } else {
                others.join(", ")
            };
            ui.label(list);
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr("Programs with unsaved documents may still ask before they quit.")).color(p.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(if log_out { tr("Log out") } else { tr("Shut down") }).color(p.on_accent),
                        )
                        .fill(p.bad),
                    )
                    .clicked()
                {
                    decided = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    decided = Some(false);
                }
            });
        });
        if modal.should_close() && decided.is_none() {
            decided = Some(false);
        }
        if let Some(go) = decided {
            self.confirm_power_off = None;
            if go && log_out {
                self.send(Command::LogOut { this_one });
            } else if go {
                self.send(Command::PowerOff { this_one });
            }
        }
    }

    /// Linux: looks every few seconds whether status icons are shown, until they are (the
    /// extension may start after TableCross, or be installed meanwhile).
    fn check_icons(&mut self) {
        #[cfg(target_os = "linux")]
        if self.tray.is_some()
            && (self.icons_missing || self.icons_checked.is_none())
            && self
                .icons_checked
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(10))
        {
            self.icons_checked = Some(Instant::now());
            let missing = crate::tray::icons_shown() == Some(false);
            if missing != self.icons_missing {
                tracing::info!("status icons shown by the desktop: {}", !missing);
            }
            self.icons_missing = missing;
        }
    }

    /// Asks where the files copied on another computer go, then fetches them.
    pub fn save_files(&mut self) {
        if self.status.files_waiting.as_ref().is_some_and(|w| !w.pasteable) {
            self.save_files_to = Some(tc_session::downloads());
        } else {
            self.show_info(tr("No files are copied on another computer."));
        }
    }

    fn save_files_dialog(&mut self, ctx: &egui::Context) {
        if self.save_files_flag.swap(false, Ordering::SeqCst) {
            self.save_files();
        }
        let Some(folder) = self.save_files_to.clone() else {
            return;
        };
        // Copied something else meanwhile
        let Some(waiting) = self.status.files_waiting.clone() else {
            self.save_files_to = None;
            return;
        };
        let p = theme::palette(ctx);
        let mut decided = None;
        let modal = egui::Modal::new(egui::Id::new("save_files")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(RichText::new(tr("Save files from the clipboard")).size(16.0).strong());
            ui.add_space(6.0);
            ui.label(i18n::trf("Copied on {}:", &[&waiting.peer]));
            ui.label(RichText::new(&waiting.label).strong());
            ui.label(
                RichText::new(i18n::trf(
                    "{} files, {}",
                    &[&waiting.count, &crate::texts::bytes(waiting.size)],
                ))
                .color(p.muted),
            );
            ui.add_space(10.0);
            ui.label(tr("Save in"));
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(folder.display().to_string()).monospace()).truncate());
                if ui.button(tr("Change …")).clicked()
                    && let Some(chosen) = rfd::FileDialog::new().set_directory(&folder).pick_folder()
                {
                    self.save_files_to = Some(chosen);
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(RichText::new(tr("Save")).color(p.on_accent)).fill(p.accent))
                    .clicked()
                {
                    decided = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    decided = Some(false);
                }
            });
        });
        if modal.should_close() && decided.is_none() {
            decided = Some(false);
        }
        if let Some(go) = decided {
            if go && let Some(folder) = self.save_files_to.clone() {
                tracing::info!("saving the copied files in {}", folder.display());
                self.send(Command::SaveFiles(folder));
                self.page = Page::Status;
            }
            self.save_files_to = None;
        }
    }

    /// Opens the Input Monitoring settings and the helper file once, if a just replaced helper lacks
    /// the permission.
    fn helper_prompt(&mut self) {
        let Some(until) = self.helper_prompt else { return };
        if Instant::now() > until {
            self.helper_prompt = None;
        } else if self.status.problems.contains(&tc_input::Problem::HelperNoAccess) {
            self.helper_prompt = None;
            #[cfg(target_os = "macos")]
            tc_session::helper_install::show_permission_place();
        }
    }

    /// macOS: Accessibility was granted after the start and only reaches a new process: start anew
    /// by ourselves instead of asking for it.
    fn restart_for_permission(&mut self, _ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        if self.status.restart_needed && !self.restart_tried {
            self.restart_tried = true;
            match crate::self_update::restart() {
                Ok(()) => {
                    tracing::info!("starting anew to use the Accessibility permission");
                    self.quit(_ctx, "restart for a permission");
                }
                Err(e) => {
                    tracing::warn!("cannot start anew by myself: {e}");
                    self.show_info(tr(
                        "Accessibility is granted now. Please quit TableCross and start it again.",
                    ));
                }
            }
        }
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        if let Ok(result) = job.try_recv() {
            self.job = None;
            match result {
                Ok(()) => self.show_info(tr("Done.")),
                Err(e) => self.show_info(i18n::trf("That did not work: {}", &[&e])),
            }
            self.send(Command::RetryInput);
        }
    }
}

/// One entry of the page list.
/// One entry of the sidebar; `symbol_color` overrides the color of the symbol (status).
fn nav_item(
    ui: &mut egui::Ui,
    symbol: &str,
    label: &str,
    selected: bool,
    symbol_color: Option<egui::Color32>,
) -> egui::Response {
    let p = theme::colors(ui);
    let width = ui.available_width() - 12.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::click());
    let rect = rect.translate(egui::vec2(6.0, 0.0));
    let (fill, fg) = if selected {
        (p.accent_soft, p.accent)
    } else if response.hovered() {
        (p.window, p.text)
    } else {
        (egui::Color32::TRANSPARENT, p.text)
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    let font = if selected {
        egui::FontId::new(14.0, theme::semibold())
    } else {
        egui::FontId::proportional(14.0)
    };
    ui.painter().text(
        rect.left_center() + egui::vec2(14.0, 0.0),
        egui::Align2::LEFT_CENTER,
        symbol,
        egui::FontId::proportional(15.0),
        symbol_color.unwrap_or(if selected { p.accent } else { p.muted }),
    );
    ui.painter().text(
        rect.left_center() + egui::vec2(40.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        fg,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.dev.frame(&ctx);
        self.apply_theme(&ctx);
        self.poll_job();
        self.update_check(&ctx);
        self.status = self.current_status();
        self.restart_for_permission(&ctx);
        // Another computer changes the arrangement: its bars at the edges here, for a moment
        if let Some((marks, at)) = &self.status.edge_marks
            && at.elapsed() < EDGES_SHOWN
        {
            crate::edges::show(
                &ctx,
                &self.status.screens,
                marks,
                theme::palette(&ctx).ok,
                crate::edges::THICK,
            );
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let peer_edges = self
            .status
            .edge_marks
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < EDGES_SHOWN);
        self.helper_prompt();
        self.poll_removal(&ctx);

        // Closing the window keeps TableCross running in the menu bar / notification area
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_flag.load(Ordering::SeqCst)
            && !self.dev.quitting_after_screenshot()
            && self.has_tray()
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            crate::tray::hide_window(&ctx);
        }
        if self.background {
            self.background = false;
        }

        let p = theme::palette(&ctx);
        egui::Panel::left("nav")
            .exact_size(232.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| self.sidebar(ui));
        if let Some((text, since)) = &self.info
            && since.elapsed() < Duration::from_secs(8)
        {
            let text = text.clone();
            egui::Panel::bottom("info")
                .exact_size(36.0)
                .frame(
                    egui::Frame::new()
                        .fill(p.accent_soft)
                        .inner_margin(egui::Margin::symmetric(24, 0)),
                )
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| ui.label(RichText::new(text).color(p.text)));
                });
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.window).inner_margin(egui::Margin {
                left: 28,
                right: 20,
                top: 18,
                bottom: 0,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 8.0);
                    if let Some(step) = self.setup_step {
                        self.setup(ui, step);
                    } else {
                        match self.page {
                            Page::Status => self.status_page(ui),
                            Page::Arrangement => self.arrangement_page(ui),
                            Page::Pairing => self.pairing_page(ui),
                            Page::Settings => self.settings_page(ui),
                            Page::About => self.about_page(ui),
                        }
                    }
                });
            });
        // Thin bars where the pointer crosses, all the time if wanted (not while others are shown)
        if self.settings.edges_always && !peer_edges && !crate::pages::arrangement::dragging_edges(&ctx) {
            let marks = crate::pages::arrangement::own_marks(&self.status);
            crate::edges::show(
                &ctx,
                &self.status.screens,
                &marks,
                theme::palette(&ctx).ok,
                crate::edges::THIN,
            );
        }
        self.power_off_dialog(&ctx);
        self.save_files_dialog(&ctx);
        // Numbers like the round trip change without events
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_session();
    }
}
