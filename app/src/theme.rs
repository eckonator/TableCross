//! Look of the app: fonts, colors for light and dark, spacing, and small building blocks (cards,
//! chips, banners) that the pages use.
//!
//! Fonts: Inter for the interface, a cut of DejaVu Sans for symbols the others lack.
//! Licenses: assets/fonts/Inter-LICENSE.txt (OFL-1.1), assets/fonts/DejaVu-LICENSE.txt.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Stroke, TextStyle,
};

/// The semibold cut of Inter, for titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        &mut fonts,
        "inter",
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    );
    add(
        &mut fonts,
        "inter-semibold",
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    );
    add(
        &mut fonts,
        "symbols",
        include_bytes!("../../assets/fonts/DejaVuSans-Symbole.ttf"),
    );
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".into());
    let mut semi = vec!["inter-semibold".to_string()];
    semi.extend(fallback);
    semi.push("symbols".into());
    fonts.families.insert(semibold(), semi);
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("symbols".into());
    }
    ctx.set_fonts(fonts);
}

/// Colors of one mode.
#[derive(Clone, Copy)]
pub struct Palette {
    pub window: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub bad: Color32,
    pub field: Color32,
}

const LIGHT: Palette = Palette {
    window: Color32::from_rgb(0xF4, 0xF6, 0xFA),
    sidebar: Color32::from_rgb(0xEA, 0xEE, 0xF5),
    card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    border: Color32::from_rgb(0xDD, 0xE2, 0xEB),
    text: Color32::from_rgb(0x17, 0x1B, 0x24),
    muted: Color32::from_rgb(0x5A, 0x63, 0x73),
    accent: Color32::from_rgb(0x2B, 0x6C, 0xD9),
    accent_soft: Color32::from_rgb(0xE1, 0xEC, 0xFC),
    on_accent: Color32::WHITE,
    ok: Color32::from_rgb(0x15, 0x80, 0x45),
    ok_soft: Color32::from_rgb(0xE0, 0xF4, 0xE8),
    warn: Color32::from_rgb(0x9A, 0x5B, 0x00),
    warn_soft: Color32::from_rgb(0xFD, 0xF1, 0xDC),
    bad: Color32::from_rgb(0xB4, 0x2B, 0x2B),
    field: Color32::from_rgb(0xF1, 0xF3, 0xF7),
};

const DARK: Palette = Palette {
    window: Color32::from_rgb(0x14, 0x17, 0x1D),
    sidebar: Color32::from_rgb(0x10, 0x12, 0x17),
    card: Color32::from_rgb(0x1C, 0x20, 0x28),
    border: Color32::from_rgb(0x2C, 0x32, 0x3D),
    text: Color32::from_rgb(0xE8, 0xEB, 0xF1),
    muted: Color32::from_rgb(0x9A, 0xA3, 0xB2),
    accent: Color32::from_rgb(0x5B, 0x95, 0xF5),
    accent_soft: Color32::from_rgb(0x1F, 0x2E, 0x4A),
    on_accent: Color32::WHITE,
    ok: Color32::from_rgb(0x4C, 0xC9, 0x84),
    ok_soft: Color32::from_rgb(0x17, 0x33, 0x25),
    warn: Color32::from_rgb(0xF0, 0xB4, 0x4C),
    warn_soft: Color32::from_rgb(0x3A, 0x2C, 0x13),
    bad: Color32::from_rgb(0xF0, 0x70, 0x70),
    field: Color32::from_rgb(0x15, 0x19, 0x20),
};

pub fn palette(ctx: &egui::Context) -> Palette {
    if ctx.global_style().visuals.dark_mode {
        DARK
    } else {
        LIGHT
    }
}

pub fn colors(ui: &egui::Ui) -> Palette {
    palette(ui.ctx())
}

/// Sizes, spacing and widget colors for both modes.
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let p = if style.visuals.dark_mode { DARK } else { LIGHT };
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(24.0, semibold())),
            (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.icon_width = 17.0;
        style.spacing.icon_width_inner = 10.0;
        let v = &mut style.visuals;
        v.panel_fill = p.window;
        v.window_fill = p.card;
        v.extreme_bg_color = p.field;
        v.faint_bg_color = p.field;
        v.override_text_color = None;
        v.weak_text_color = Some(p.muted);
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent_soft;
        v.selection.stroke = Stroke::new(1.5, p.accent);
        v.window_stroke = Stroke::new(1.0, p.border);
        let radius = CornerRadius::same(5);
        for (w, fill) in [
            (&mut v.widgets.noninteractive, p.card),
            (&mut v.widgets.inactive, p.field),
            (&mut v.widgets.hovered, p.accent_soft),
            (&mut v.widgets.active, p.accent_soft),
            (&mut v.widgets.open, p.accent_soft),
        ] {
            w.corner_radius = radius;
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        // Controls get a clearer edge than cards, so check boxes and fields stand out
        let control_edge = if v.dark_mode {
            Color32::from_rgb(0x46, 0x4E, 0x5C)
        } else {
            Color32::from_rgb(0xB3, 0xBB, 0xC9)
        };
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, control_edge);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
        v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
        v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    });
}

/// A titled section on a light surface.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = colors(ui);
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12.0)
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).font(FontId::new(16.0, semibold())).color(p.text));
                ui.add_space(8.0);
            }
            add(ui)
        })
        .inner
}

/// A small rounded label, e.g. "connected".
pub fn chip(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(fg));
        });
}

/// The main action of a section.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = colors(ui);
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .color(p.on_accent)
                .font(FontId::new(14.0, semibold())),
        )
        .fill(p.accent)
        .stroke(Stroke::NONE)
        .corner_radius(8.0),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Ok,
    Warn,
}

/// A note with a colored edge; `add` puts buttons below the text.
pub fn banner(ui: &mut egui::Ui, tone: Tone, text: &str, add: impl FnOnce(&mut egui::Ui)) {
    let p = colors(ui);
    let (edge, fill, symbol) = match tone {
        Tone::Info => (p.accent, p.accent_soft, "ℹ"),
        Tone::Ok => (p.ok, p.ok_soft, "✔"),
        Tone::Warn => (p.warn, p.warn_soft, "⚠"),
    };
    let response = egui::Frame::new()
        .fill(fill)
        .corner_radius(10.0)
        .inner_margin(Margin {
            left: 18,
            right: 14,
            top: 12,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(symbol).size(16.0).color(edge));
                ui.vertical(|ui| {
                    ui.label(RichText::new(text).color(p.text));
                    ui.horizontal_wrapped(add);
                });
            });
        })
        .response;
    let rect = response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 4.0, rect.max.y)),
        CornerRadius {
            nw: 10,
            sw: 10,
            ne: 0,
            se: 0,
        },
        edge,
    );
}

/// A computer screen drawn as a monitor with a stand; `inner` is the content area.
pub fn monitor(painter: &egui::Painter, rect: egui::Rect, frame: Color32, screen: Color32) -> egui::Rect {
    let stand_h = (rect.height() * 0.16).clamp(6.0, 16.0);
    let body = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - stand_h));
    painter.rect_filled(body, 8.0, frame);
    let inner = body.shrink(5.0);
    painter.rect_filled(inner, 5.0, screen);
    let neck_w = (rect.width() * 0.08).max(6.0);
    let foot_w = (rect.width() * 0.32).max(20.0);
    let cx = rect.center().x;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(cx - neck_w / 2.0, body.max.y),
            egui::pos2(cx + neck_w / 2.0, rect.max.y - 3.0),
        ),
        0.0,
        frame,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(cx - foot_w / 2.0, rect.max.y - 4.0),
            egui::pos2(cx + foot_w / 2.0, rect.max.y),
        ),
        2.0,
        frame,
    );
    inner
}
