//! The window: top bar, navigation, pages. Pages only draw and send commands to the session.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, ViewportCommand};
use tc_net::Fingerprint;
use tc_session::{Command, SessionConfig, SessionHandle, Status};

use crate::devtools::DevTools;
use crate::i18n::{self, tr};
use crate::settings::{Settings, ThemeSetting};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Status,
    Computers,
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
    pub pair_target: Option<Fingerprint>,
    pub confirm_unpair: Option<Fingerprint>,
    pub info: Option<(String, Instant)>,
    pub job: Option<Job>,
    pub isolated: bool,
    pub log_file: Option<PathBuf>,
    quit_flag: Arc<AtomicBool>,
    tray: Option<crate::tray::Tray>,
    dev: DevTools,
    background: bool,
    logo: Option<egui::TextureHandle>,
    /// A newer version found on GitHub.
    pub new_version: Option<crate::update::NewVersion>,
    update_job: Option<std::sync::mpsc::Receiver<Option<crate::update::NewVersion>>>,
    update_checked: Option<Instant>,
    /// Installing a new version from inside the app, and how far it got.
    self_update: Option<std::sync::mpsc::Receiver<crate::self_update::Progress>>,
    pub update_progress: Option<crate::self_update::Progress>,
}

/// How often to look for a new version while running.
const UPDATE_EVERY: Duration = Duration::from_secs(12 * 3600);

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
            pair_target: None,
            confirm_unpair: None,
            info: None,
            job: None,
            isolated,
            log_file,
            quit_flag: Arc::new(AtomicBool::new(false)),
            tray: None,
            dev,
            background,
            logo,
            new_version: None,
            update_job: None,
            update_checked: None,
            self_update: None,
            update_progress: None,
        };
        if app.demo.is_none() {
            app.start_session(&ctx);
        }
        app.page = match app.dev.page.as_deref() {
            Some("computers") => Page::Computers,
            Some("settings") => Page::Settings,
            Some("about") => Page::About,
            _ => Page::Status,
        };
        let first_start = !app.settings.setup_done && app.current_status().peers.is_empty();
        app.setup_step = app
            .dev
            .setup_step
            .map(|s| s.clamp(1, 3) - 1)
            .or((first_start && app.dev.page.is_none()).then_some(0));
        if !app.dev.quitting_after_screenshot() {
            app.tray = crate::tray::Tray::new(&ctx, app.session.clone(), app.quit_flag.clone());
            // Only with a status icon: otherwise the window could not be found again
            if app.tray.is_some() {
                crate::tray::menu_bar_only();
                if !background {
                    crate::tray::bring_to_front();
                }
            }
        }
        if background && app.has_tray() {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
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

    fn stop_session(&mut self) {
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
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            theme::chip(ui, &text, fg, bg);
        });
        ui.add_space(18.0);
        for (page, symbol, label) in [
            (Page::Status, "◉", tr("Status")),
            (Page::Computers, "▣", tr("Computers")),
            (Page::Settings, "⚙", tr("Settings")),
            (Page::About, "ℹ", tr("About")),
        ] {
            let selected = self.page == page && self.setup_step.is_none();
            if nav_item(ui, symbol, label, selected).clicked() {
                self.page = page;
                self.setup_step = None;
            }
        }
        ui.add_space(6.0);
        if nav_item(ui, "✦", tr("Setup"), self.setup_step.is_some()).clicked() {
            self.setup_step = Some(0);
        }
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
            self.new_version = result;
            self.update_job = None;
        }
        if let Some(job) = &self.self_update {
            while let Ok(progress) = job.try_recv() {
                if progress == crate::self_update::Progress::Restart {
                    // The new version starts as soon as this one has ended
                    self.quit_flag.store(true, Ordering::SeqCst);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                self.update_progress = Some(progress);
            }
        }
        let due = self.update_checked.is_none_or(|t| t.elapsed() >= UPDATE_EVERY);
        if self.settings.update_check == Some(true) && self.demo.is_none() && self.update_job.is_none() && due {
            self.update_checked = Some(Instant::now());
            self.update_job = Some(crate::update::check_in_background(ctx.clone()));
        }
    }

    pub fn install_update(&mut self, ctx: &egui::Context) {
        if let Some(url) = self.new_version.as_ref().and_then(|n| n.asset.clone()) {
            self.update_progress = Some(crate::self_update::Progress::Downloading);
            self.self_update = Some(crate::self_update::start(url, ctx.clone()));
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
fn nav_item(ui: &mut egui::Ui, symbol: &str, label: &str, selected: bool) -> egui::Response {
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
        if selected { p.accent } else { p.muted },
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

        // Closing the window keeps TableCross running in the menu bar / notification area
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_flag.load(Ordering::SeqCst)
            && !self.dev.quitting_after_screenshot()
            && self.has_tray()
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        if self.background {
            self.background = false;
        }

        let p = theme::palette(&ctx);
        egui::Panel::left("nav")
            .exact_size(212.0)
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
                            Page::Computers => self.computers_page(ui),
                            Page::Settings => self.settings_page(ui),
                            Page::About => self.about_page(ui),
                        }
                    }
                });
            });
        // Numbers like the round trip change without events
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_session();
    }
}
