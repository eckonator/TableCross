//! Settings: language, appearance, start with the system, network, keyboard shortcuts.

use eframe::egui::{self, RichText};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::settings::{EdgeSetting, LangSetting, ThemeSetting, host_name};
use tc_session::Command;

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(ui, tr("Settings"), tr("How TableCross behaves on this computer."));
        let mut changed = false;
        card(ui, tr("General"), |ui| {
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

                    #[cfg(target_os = "macos")]
                    {
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
                    }
                });
        });
        ui.add_space(14.0);
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
        card(ui, tr("Remove TableCross"), |ui| self.removal_card(ui));
        if changed {
            self.save_settings();
        }
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

fn edge_text(e: EdgeSetting) -> &'static str {
    match e {
        EdgeSetting::AtOnce => tr("switch at once"),
        EdgeSetting::Push => tr("after a moment of pushing"),
        EdgeSetting::DoubleTap => tr("after touching twice"),
    }
}
