//! Settings in tabs: general, pointer and keyboard, Mac, network, troubleshooting.

use eframe::egui::{self, RichText};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::settings::{EdgeSetting, LangSetting, SettingsTab, ThemeSetting, host_name};
use crate::theme;
use tc_session::Command;

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(ui, tr("Settings"), tr("How TableCross behaves on this computer."));
        let mut changed = self.settings_tabs(ui);
        ui.add_space(12.0);
        changed |= match self.settings.settings_tab {
            SettingsTab::General => self.general_settings(ui),
            SettingsTab::Pointer => self.pointer_settings(ui),
            SettingsTab::Mac => self.mac_settings(ui),
            SettingsTab::Network => self.network_settings(ui),
            SettingsTab::Troubleshooting => self.troubleshooting_settings(ui),
        };
        if changed {
            self.save_settings();
        }
    }

    /// The row of tabs; whether another one was chosen.
    fn settings_tabs(&mut self, ui: &mut egui::Ui) -> bool {
        let mut tabs = vec![
            (SettingsTab::General, tr("General")),
            (SettingsTab::Pointer, tr("Pointer & keyboard")),
        ];
        if cfg!(target_os = "macos") {
            tabs.push((SettingsTab::Mac, "Mac"));
        }
        tabs.push((SettingsTab::Network, tr("Network")));
        tabs.push((SettingsTab::Troubleshooting, tr("Troubleshooting")));
        // The Mac tab of a settings file from a Mac does not exist elsewhere
        if !tabs.iter().any(|(t, _)| *t == self.settings.settings_tab) {
            self.settings.settings_tab = SettingsTab::General;
        }
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (tab, label) in tabs {
                if settings_tab(ui, label, self.settings.settings_tab == tab).clicked() {
                    chosen = Some(tab);
                }
            }
        });
        match chosen {
            Some(tab) if tab != self.settings.settings_tab => {
                self.settings.settings_tab = tab;
                true
            }
            _ => false,
        }
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        card(ui, "", |ui| {
            egui::Grid::new("general")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Language"));
                    let before = self.settings.language;
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(match self.settings.language {
                            LangSetting::System => tr("Like the system"),
                            LangSetting::De => "Deutsch",
                            LangSetting::En => "English",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.settings.language,
                                LangSetting::System,
                                tr("Like the system"),
                            );
                            ui.selectable_value(&mut self.settings.language, LangSetting::De, "Deutsch");
                            ui.selectable_value(&mut self.settings.language, LangSetting::En, "English");
                        });
                    if before != self.settings.language {
                        self.language_changed();
                        changed = true;
                    }
                    ui.end_row();

                    ui.label(tr("Appearance"));
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(match self.settings.theme {
                            ThemeSetting::System => tr("Like the system"),
                            ThemeSetting::Light => tr("Light"),
                            ThemeSetting::Dark => tr("Dark"),
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut self.settings.theme, ThemeSetting::System, tr("Like the system"))
                                .changed();
                            changed |= ui
                                .selectable_value(&mut self.settings.theme, ThemeSetting::Light, tr("Light"))
                                .changed();
                            changed |= ui
                                .selectable_value(&mut self.settings.theme, ThemeSetting::Dark, tr("Dark"))
                                .changed();
                        });
                    ui.end_row();

                    ui.label(tr("Updates"));
                    ui.horizontal(|ui| {
                        let mut check = self.settings.update_check == Some(true);
                        if ui.checkbox(&mut check, tr("Look for new versions on GitHub")).changed() {
                            self.settings.update_check = Some(check);
                            changed = true;
                        }
                        self.check_button(ui);
                    });
                    ui.end_row();
                    ui.label("");
                    changed |= ui
                        .checkbox(
                            &mut self.settings.update_with_peers,
                            tr("Update together with paired computers"),
                        )
                        .on_hover_text(tr(
                            "When a paired computer runs a newer version, this one fetches the same version from \
                             GitHub and installs it.",
                        ))
                        .changed();
                    ui.end_row();

                    ui.label(tr("Start"));
                    if ui
                        .checkbox(&mut self.settings.autostart, tr("Start TableCross with the system"))
                        .changed()
                    {
                        changed = true;
                        if !self.isolated
                            && let Err(e) = crate::autostart::set(self.settings.autostart)
                        {
                            self.show_info(trf("Could not change the start with the system: {}", &[&e]));
                        }
                    }
                    ui.end_row();

                    ui.label(tr("Sleep"));
                    if ui
                        .checkbox(
                            &mut self.settings.stay_awake,
                            tr("Keep this computer from going to sleep"),
                        )
                        .on_hover_text(tr(
                            "While TableCross runs. A computer that sleeps drops out, and with it the way over \
                             it to the computers behind.",
                        ))
                        .changed()
                    {
                        changed = true;
                        self.apply_stay_awake();
                    }
                    ui.end_row();
                    ui.label("");
                    if ui
                        .add_enabled(
                            self.settings.stay_awake,
                            egui::Checkbox::new(&mut self.settings.screen_stays_on, tr("Keep the screen on as well")),
                        )
                        .changed()
                    {
                        changed = true;
                        self.apply_stay_awake();
                    }
                    ui.end_row();
                });
        });
        changed
    }

    fn pointer_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        card(ui, "", |ui| {
            egui::Grid::new("pointer")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Top and bottom"));
                    let before = self.settings.edges;
                    egui::ComboBox::from_id_salt("edges")
                        .selected_text(edge_text(self.settings.edges))
                        .show_ui(ui, |ui| {
                            for e in [EdgeSetting::AtOnce, EdgeSetting::Push, EdgeSetting::DoubleTap] {
                                ui.selectable_value(&mut self.settings.edges, e, edge_text(e));
                            }
                        })
                        .response
                        .on_hover_text(tr(
                            "The Dock and the menu bar are at these edges: switching only after pushing or a \
                             double tap keeps them from switching by accident. Left and right always switch at once.",
                        ));
                    if before != self.settings.edges {
                        changed = true;
                        self.send(Command::EdgeRule(self.settings.edges.rule()));
                    }
                    ui.end_row();

                    ui.label(tr("Corners"));
                    let response = ui
                        .add(
                            egui::Slider::new(&mut self.settings.corner, 0..=100)
                                .suffix(" pt")
                                .text(tr("no switching this close to a corner")),
                        )
                        .on_hover_text(tr(
                            "At left and right edges, so the pointer can reach things in the corners (0: off).",
                        ));
                    if response.changed() {
                        changed = true;
                        self.send(Command::Corner(f64::from(self.settings.corner)));
                    }
                    ui.end_row();

                    #[cfg(any(target_os = "macos", windows))]
                    {
                        ui.label(tr("Keyboard layout"));
                        if ui
                            .checkbox(
                                &mut self.settings.follow_layout,
                                tr("Use the layout of the computer in control"),
                            )
                            .on_hover_text(tr(
                                "While another computer controls this one, its keyboard layout (German, Swiss \
                                 German, US …) is switched on here, so every key gives the same character. The \
                                 layout used before comes back afterwards.",
                            ))
                            .changed()
                        {
                            changed = true;
                            self.send(Command::FollowLayout(self.settings.follow_layout));
                        }
                        ui.end_row();
                    }

                    ui.label(tr("Crossings"));
                    changed |= ui
                        .checkbox(
                            &mut self.settings.edges_always,
                            tr("Always show where the pointer crosses"),
                        )
                        .on_hover_text(tr(
                            "Thin green bars at the edges of the screens where the pointer goes to another computer. \
                             Otherwise they appear only while the arrangement is changed.",
                        ))
                        .changed();
                    ui.end_row();

                    ui.label(tr("Offline computers"));
                    if ui
                        .checkbox(&mut self.settings.skip_offline, tr("Skip computers that are offline"))
                        .on_hover_text(tr(
                            "When a computer in the arrangement is off or not connected, the pointer goes on to \
                             the next computer behind it in the same direction.",
                        ))
                        .changed()
                    {
                        changed = true;
                        self.send(Command::SkipOffline(self.settings.skip_offline));
                    }
                    ui.end_row();
                });
        });
        ui.add_space(14.0);
        card(ui, tr("Keyboard shortcuts"), |ui| {
            for (keys, what) in [
                (
                    tr("Ctrl + Shift + Alt + Esc"),
                    tr("Release everything: all keys up, mouse and keyboard back to this computer"),
                ),
                (
                    tr("Ctrl + Shift + Alt + arrow key"),
                    tr("Go to the computer on that side"),
                ),
                (
                    tr("Ctrl + Shift + Alt + 1 … 9"),
                    tr("Go to the computer with that number (see Arrangement)"),
                ),
                (tr("Ctrl + Shift + Alt + L"), tr("Lock the screens of all computers")),
                (
                    tr("Ctrl + Shift + Alt + F"),
                    tr("Keep the pointer on this computer, or let it go again"),
                ),
            ] {
                ui.label(RichText::new(keys).monospace().strong());
                ui.label(what);
                ui.add_space(4.0);
            }
        });
        changed
    }

    /// The Mac tab is not offered elsewhere.
    #[cfg(not(target_os = "macos"))]
    fn mac_settings(&mut self, _ui: &mut egui::Ui) -> bool {
        false
    }

    #[cfg(target_os = "macos")]
    fn mac_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        card(ui, "", |ui| {
            egui::Grid::new("mac")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Login screen"));
                    if ui
                        .checkbox(
                            &mut self.settings.login_screen,
                            tr("Can be controlled on the login screen"),
                        )
                        .on_hover_text(tr(
                            "Before anyone logs in, this Mac can be used from the other computers as well, for \
                                 example to type the password. Needs the system helper, and tablecross-loginwindow \
                                 needs the Accessibility permission (the status page shows how).",
                        ))
                        .changed()
                    {
                        changed = true;
                        self.send(Command::LoginScreen(self.settings.login_screen));
                    }
                    ui.end_row();

                    ui.label(tr("AirDrop radio"));
                    if ui
                        .checkbox(
                            &mut self.settings.quiet_radio,
                            tr("Pause AirDrop and Handoff while sharing"),
                        )
                        .on_hover_text(tr(
                            "Their radio link (AWDL) leaves the Wi-Fi channel every few seconds, and the \
                                 pointer stutters meanwhile. The system helper pauses it while this Mac controls \
                                 or is controlled, and lets it run again afterwards. AirDrop, Handoff and AirPlay \
                                 to nearby devices wait until then.",
                        ))
                        .changed()
                    {
                        changed = true;
                        self.send(Command::QuietRadio(self.settings.quiet_radio));
                    }
                    ui.end_row();

                    ui.label(tr("Universal Control"));
                    if ui
                        .checkbox(
                            &mut self.settings.universal_control_off,
                            tr("Switch off Universal Control of macOS"),
                        )
                        .on_hover_text(tr(
                            "Universal Control also moves the pointer to other Macs at the screen edges. \
                                 TableCross switches it off at start so the two do not fight over the pointer.",
                        ))
                        .changed()
                    {
                        changed = true;
                        if self.settings.universal_control_off
                            && !self.isolated
                            && tc_input::universal_control::is_on()
                            && let Err(e) = tc_input::universal_control::turn_off()
                        {
                            self.show_info(trf("Could not switch off Universal Control: {}", &[&e]));
                        }
                    }
                    ui.end_row();

                    ui.label(tr("Handoff"));
                    if ui
                        .checkbox(&mut self.settings.handoff_off, tr("Switch off Handoff of macOS"))
                        .on_hover_text(tr(
                            "With Handoff, macOS carries the clipboard to your other Macs as well and sends what \
                                 TableCross shares back as new: pasting then waits for the other Mac. TableCross \
                                 switches it off at start. This also stops the clipboard with an iPhone or iPad.",
                        ))
                        .changed()
                    {
                        changed = true;
                        if self.settings.handoff_off
                            && !self.isolated
                            && tc_input::handoff::is_on()
                            && let Err(e) = tc_input::handoff::turn_off()
                        {
                            self.show_info(trf("Could not switch off Handoff: {}", &[&e]));
                        }
                    }
                    ui.end_row();
                });
        });
        #[cfg(target_os = "macos")]
        self.network_filter_hint(ui);
        changed
    }

    /// macOS: a network filter that asks per program is installed. Two things go wrong with it
    /// unless set up: its questions cannot be answered from another computer (it ignores
    /// injected clicks by default), and on the login screen no one answers them at all.
    #[cfg(target_os = "macos")]
    fn network_filter_hint(&mut self, ui: &mut egui::Ui) {
        if !std::path::Path::new("/Applications/Little Snitch.app").exists() {
            return;
        }
        ui.add_space(14.0);
        card(ui, tr("Little Snitch is installed"), |ui| {
            let muted = theme::colors(ui).muted;
            ui.label(
                RichText::new(tr(
                    "It ignores clicks and keys from other computers, so its questions can only be answered at \
                     this Mac. To answer them from another computer as well, switch on “Allow GUI Scripting \
                     access to Little Snitch” in its settings under Security.",
                ))
                .color(muted),
            );
            if self.settings.login_screen {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr(
                        "For the login screen it needs both: GUI Scripting access as above, and a rule that \
                         allows tablecross-loginwindow in the local network, incoming and outgoing (no one can \
                         answer its question there, so it blocks the program silently).",
                    ))
                    .color(muted),
                );
                if ui.button(tr("Show tablecross-loginwindow in the Finder")).clicked() {
                    tc_session::helper_install::show_login_program();
                }
            }
        });
    }

    fn network_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        card(ui, tr("Network"), |ui| {
            egui::Grid::new("network")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Name of this computer"));
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.settings.name)
                                .hint_text(host_name())
                                .margin(egui::Margin::symmetric(8, 6)),
                        )
                        .changed();
                    ui.end_row();
                    ui.label(tr("UDP port"));
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.settings.port).range(1024..=65535))
                        .changed();
                    ui.end_row();
                    ui.label(tr("Search"));
                    changed |= ui
                        .checkbox(
                            &mut self.settings.discovery,
                            tr("Find computers on the network automatically"),
                        )
                        .changed();
                    ui.end_row();
                });
            if self.needs_restart() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Takes effect after a restart of the connection.")).weak());
                    if ui.button(tr("Restart now")).clicked() {
                        let ctx = ui.ctx().clone();
                        self.restart_session(&ctx);
                    }
                });
            }
        });
        ui.add_space(14.0);
        card(ui, tr("Clipboard"), |ui| {
            changed |= ui
                .checkbox(
                    &mut self.settings.clipboard,
                    tr("Share the clipboard with the other computers"),
                )
                .changed();
            ui.add_enabled_ui(self.settings.clipboard, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Largest content"));
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.settings.clipboard_max_mb).range(1..=64).suffix(" MB"))
                        .changed();
                });
                ui.label(
                    RichText::new(tr(
                        "Text, formatted text and images. Passwords copied from a password manager stay on this computer.",
                    ))
                    .weak(),
                );
            });
            if self.needs_restart() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Takes effect after a restart of the connection.")).weak());
                    if ui.button(tr("Restart now")).clicked() {
                        let ctx = ui.ctx().clone();
                        self.restart_session(&ctx);
                    }
                });
            }
        });
        changed
    }

    fn troubleshooting_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        card(ui, tr("Troubleshooting"), |ui| {
            if ui
                .checkbox(
                    &mut self.settings.collect_logs,
                    tr("Collect the logs of the other computers here"),
                )
                .changed()
            {
                changed = true;
                self.send(Command::CollectLogs(self.settings.collect_logs));
            }
            ui.label(
                RichText::new(tr(
                    "The connected computers send their log to this one while it is on, so a problem can be \
                     looked at in one place. Only on paired computers, over the encrypted connection; the logs \
                     contain no typed text.",
                ))
                .weak(),
            );
            let folder = tc_session::peer_logs::folder(&self.dir);
            if ui.button(tr("Open the folder with the collected logs")).clicked() {
                let _ = std::fs::create_dir_all(&folder);
                super::open(&folder.display().to_string());
            }
        });
        ui.add_space(14.0);
        card(ui, tr("Files"), |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr("Open settings folder")).clicked() {
                    super::open(&self.dir.display().to_string());
                }
                if let Some(log) = self.log_file.clone()
                    && ui.button(tr("Show log file")).clicked()
                {
                    super::open(&log.display().to_string());
                }
            });
        });
        ui.add_space(14.0);
        card(ui, tr("Remove TableCross"), |ui| self.removal_card(ui));
        changed
    }

    /// "Check for updates", with a spinner while it runs.
    pub fn check_button(&mut self, ui: &mut egui::Ui) {
        if self.checking_updates() {
            ui.spinner();
        } else if ui.button(tr("Check for updates")).clicked() {
            let ctx = ui.ctx().clone();
            self.check_updates_now(&ctx);
        }
    }

    fn removal_card(&mut self, ui: &mut egui::Ui) {
        let p = crate::theme::colors(ui);
        ui.label(
            RichText::new(tr(
                "Removes TableCross from this computer with everything it set up: the system helper, the \
                 permissions, starting with the system and the program itself.",
            ))
            .color(p.muted),
        );
        ui.checkbox(
            &mut self.remove_settings,
            tr("Also delete settings and pairings (the other computers forget this one)"),
        );
        ui.add_space(6.0);
        if self.removing() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Removing TableCross …"));
            });
        } else if self.confirm_remove {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Really remove TableCross?")).color(p.bad));
                if ui
                    .add(egui::Button::new(RichText::new(tr("Yes, remove")).color(p.on_accent)).fill(p.bad))
                    .clicked()
                {
                    self.start_removal();
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.confirm_remove = false;
                }
            });
        } else if ui
            .add_enabled(self.demo.is_none(), egui::Button::new(tr("Remove TableCross …")))
            .clicked()
        {
            self.confirm_remove = true;
        }
    }
}

/// One tab of the settings page.
fn settings_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let p = theme::colors(ui);
    let font = if selected {
        egui::FontId::new(14.0, theme::semibold())
    } else {
        egui::FontId::proportional(14.0)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, if selected { p.accent } else { p.text });
    let size = galley.size() + egui::vec2(28.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if selected {
        p.accent_soft
    } else if response.hovered() {
        p.window
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, p.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn edge_text(e: EdgeSetting) -> &'static str {
    match e {
        EdgeSetting::AtOnce => tr("switch at once"),
        EdgeSetting::Push => tr("after a moment of pushing"),
        EdgeSetting::DoubleTap => tr("after touching twice"),
    }
}
