//! Settings: language, appearance, start with the system, network, keyboard shortcuts.

use eframe::egui::{self, RichText};

use super::card;
use crate::app::App;
use crate::i18n::{tr, trf};
use crate::settings::{LangSetting, ThemeSetting, host_name};

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
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
                });
        });
        ui.add_space(8.0);
        card(ui, tr("Network"), |ui| {
            egui::Grid::new("network")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Name of this computer"));
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut self.settings.name).hint_text(host_name()))
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
        ui.add_space(8.0);
        card(ui, tr("Keyboard shortcuts"), |ui| {
            egui::Grid::new("shortcuts")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new(tr("Ctrl + Shift + Alt + Esc")).monospace());
                    ui.label(tr(
                        "Release everything: all keys up, mouse and keyboard back to this computer",
                    ));
                    ui.end_row();
                    ui.label(RichText::new(tr("Ctrl + Shift + Alt + arrow key")).monospace());
                    ui.label(tr("Go to the computer on that side"));
                    ui.end_row();
                });
        });
        ui.add_space(8.0);
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
        if changed {
            self.save_settings();
        }
    }
}
