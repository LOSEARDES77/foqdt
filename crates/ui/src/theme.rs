//! Look and feel: palette, egui style, and the small widgets the app is built from.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Ui, vec2};
use std::ops::RangeInclusive;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub widget: Color32,
    pub hover: Color32,
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub danger: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0e, 0x10, 0x14),
    card: Color32::from_rgb(0x16, 0x19, 0x20),
    widget: Color32::from_rgb(0x1f, 0x23, 0x2c),
    hover: Color32::from_rgb(0x29, 0x2e, 0x39),
    input: Color32::from_rgb(0x0b, 0x0d, 0x11),
    border: Color32::from_rgb(0x2a, 0x2f, 0x3a),
    text: Color32::from_rgb(0xe8, 0xea, 0xf0),
    weak: Color32::from_rgb(0x8b, 0x92, 0xa3),
    accent: Color32::from_rgb(0x7c, 0x6c, 0xff),
    success: Color32::from_rgb(0x3d, 0xd6, 0x8c),
    danger: Color32::from_rgb(0xff, 0x5c, 0x7a),
};

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xf4, 0xf5, 0xf8),
    card: Color32::WHITE,
    widget: Color32::from_rgb(0xee, 0xf0, 0xf4),
    hover: Color32::from_rgb(0xe3, 0xe6, 0xed),
    input: Color32::from_rgb(0xf7, 0xf8, 0xfa),
    border: Color32::from_rgb(0xe1, 0xe4, 0xea),
    text: Color32::from_rgb(0x14, 0x16, 0x1b),
    weak: Color32::from_rgb(0x5e, 0x65, 0x73),
    accent: Color32::from_rgb(0x5b, 0x4b, 0xeb),
    success: Color32::from_rgb(0x12, 0x9e, 0x5c),
    danger: Color32::from_rgb(0xd9, 0x2d, 0x50),
};

impl Palette {
    pub fn of(ui: &Ui) -> Palette {
        if ui.visuals().dark_mode { DARK } else { LIGHT }
    }
}

/// Install the dark and light styles; egui follows the system theme until the user picks one.
pub fn apply(ctx: &egui::Context) {
    for (theme, p) in [(egui::Theme::Dark, DARK), (egui::Theme::Light, LIGHT)] {
        let mut v = if theme == egui::Theme::Dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = p.bg;
        v.window_fill = p.card;
        v.window_stroke = Stroke::new(1.0, p.border);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = p.input;
        v.faint_bg_color = p.widget;
        v.code_bg_color = p.widget;
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.card;
        w.noninteractive.weak_bg_fill = p.card;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        for (state, fill, stroke) in [
            (&mut w.inactive, p.widget, Stroke::NONE),
            (&mut w.hovered, p.hover, Stroke::new(1.0, p.border)),
            (&mut w.open, p.hover, Stroke::new(1.0, p.border)),
            (&mut w.active, p.hover, Stroke::new(1.0, p.accent)),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = stroke;
            state.fg_stroke = Stroke::new(1.5, p.text);
            state.expansion = 0.0;
        }
        for state in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.open,
            &mut w.active,
        ] {
            state.corner_radius = CornerRadius::same(10);
        }
        ctx.set_visuals_of(theme, v);
    }
    ctx.all_styles_mut(|s| {
        use egui::{FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Heading, FontId::proportional(22.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = vec2(10.0, 10.0);
        s.spacing.button_padding = vec2(14.0, 7.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.slider_rail_height = 6.0;
        // Selectable labels swallow drags, so swiping over a card selected text instead of scrolling.
        s.interaction.selectable_labels = false;
    });
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::of(ui);
    let r = egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(16)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner;
    ui.add_space(4.0);
    r
}

/// Small upper-case card title.
pub fn section(ui: &mut Ui, title: &str) {
    let p = Palette::of(ui);
    ui.label(
        RichText::new(title.to_uppercase())
            .small()
            .strong()
            .extra_letter_spacing(1.2)
            .color(p.weak),
    );
}

/// Full-width accent button.
pub fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    let p = Palette::of(ui);
    let button = egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
        .fill(p.accent)
        .stroke(Stroke::NONE)
        .corner_radius(10)
        .min_size(vec2(ui.available_width(), 40.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Pill-shaped segmented control.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
    min_width: f32,
) {
    let p = Palette::of(ui);
    egui::Frame::new()
        .fill(p.widget)
        .corner_radius(12)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (v, label) in options {
                let on = *value == *v;
                let text =
                    RichText::new(*label)
                        .strong()
                        .color(if on { Color32::WHITE } else { p.weak });
                let b = egui::Button::new(text)
                    .fill(if on { p.accent } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(9)
                    .min_size(vec2(min_width, 28.0));
                if ui
                    .add(b)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    *value = *v;
                }
            }
        });
}

/// A label with its current value on the right, over a full-width slider.
pub fn slider_row<N: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut N,
    range: RangeInclusive<N>,
    fmt: impl Fn(N) -> String,
) -> egui::Response {
    let p = Palette::of(ui);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(fmt(*value)).strong().color(p.text));
        });
    });
    ui.add_space(-6.0);
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.spacing_mut().interact_size.y = 18.0; // handle size follows this
        ui.add(egui::Slider::new(value, range).show_value(false))
    })
    .inner
}

/// Two equal columns of [`stat`]s.
pub fn stat_grid(id: &str, ui: &Ui) -> egui::Grid {
    let gap = 20.0;
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([gap, 12.0])
        .min_col_width((ui.available_width() - gap) / 2.0)
}

pub fn stat(ui: &mut Ui, label: &str, value: String) {
    let p = Palette::of(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).small().color(p.weak));
        ui.label(RichText::new(value).strong().color(p.text));
    });
}

/// "● LABEL · detail" status line.
pub fn badge(ui: &mut Ui, color: Color32, label: &str, detail: &str) {
    let p = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let pulse = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.25 + 0.75;
        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 4.0, color.gamma_multiply(pulse));
        ui.label(
            RichText::new(label)
                .small()
                .strong()
                .extra_letter_spacing(1.2)
                .color(color),
        );
        ui.label(RichText::new(format!("·  {detail}")).small().color(p.weak));
    });
}

/// Centered icon (a spinner for `None`), title and hint for an empty pane.
pub fn empty_state(ui: &mut Ui, max: egui::Vec2, icon: Option<Icon>, title: &str, body: &str) {
    let p = Palette::of(ui);
    ui.allocate_ui_with_layout(max, egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.add_space((max.y * 0.5 - 80.0).max(16.0));
        match icon {
            Some(icon) => {
                let (rect, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
                paint_icon(ui.painter(), icon, rect, p.weak.gamma_multiply(0.7), p.bg);
            }
            None => {
                ui.add(egui::Spinner::new().size(40.0).color(p.accent));
            }
        }
        ui.label(RichText::new(title).size(20.0).strong().color(p.text));
        ui.label(RichText::new(body).color(p.weak));
    });
}

/// Clickable drop target; also lights up while a file is dragged over the window.
pub fn drop_zone(ui: &mut Ui, title: &str, hint: &str) -> egui::Response {
    let p = Palette::of(ui);
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 128.0), egui::Sense::click());
    let hot = resp.hovered() || ui.input(|i| !i.raw.hovered_files.is_empty());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        14,
        if hot {
            p.accent.gamma_multiply(0.12)
        } else {
            p.input
        },
    );
    painter.rect_stroke(
        rect,
        14,
        Stroke::new(1.5, if hot { p.accent } else { p.border }),
        egui::StrokeKind::Inside,
    );
    let c = rect.center();
    let icon = egui::Rect::from_center_size(c - vec2(0.0, 28.0), vec2(30.0, 30.0));
    paint_icon(
        painter,
        Icon::Folder,
        icon,
        if hot { p.accent } else { p.weak },
        p.input,
    );
    painter.text(
        c + vec2(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(15.0),
        p.text,
    );
    painter.text(
        c + vec2(0.0, 32.0),
        egui::Align2::CENTER_CENTER,
        hint,
        egui::FontId::proportional(12.0),
        p.weak,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Full-window hint while a file is dragged over the app.
pub fn drop_overlay(ui: &Ui) {
    let p = Palette::of(ui);
    let ctx = ui.ctx();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("drop-overlay"),
    ));
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0, p.bg.gamma_multiply(0.85));
    painter.rect_stroke(
        rect.shrink(16.0),
        20,
        Stroke::new(2.0, p.accent),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to send",
        egui::FontId::proportional(28.0),
        p.accent,
    );
}

/// Sun/moon button that flips between the light and dark theme.
pub fn theme_toggle(ui: &mut Ui) {
    let p = Palette::of(ui);
    let dark = ui.visuals().dark_mode;
    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::click());
    let fill = if resp.hovered() { p.hover } else { p.widget };
    ui.painter().rect_filled(rect, 10, fill);
    paint_icon(
        ui.painter(),
        if dark { Icon::Sun } else { Icon::Moon },
        rect.shrink(9.0),
        p.text,
        fill,
    );
    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if dark { "Light theme" } else { "Dark theme" })
        .clicked()
    {
        ui.ctx().set_theme(if dark {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        });
    }
}

/// App mark: an accent tile with three QR finder patterns.
pub fn logo(ui: &mut Ui, size: f32) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, (size * 0.28) as u8, p.accent);
    finders(painter, rect, Color32::WHITE);
}

/// Three QR finder patterns and a data dot, inset in `rect`.
fn finders(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let size = rect.width();
    let m = size * 0.2;
    let f = size * 0.24;
    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)] {
        let min = rect.min + vec2(m + dx * (size - 2.0 * m - f), m + dy * (size - 2.0 * m - f));
        let r = egui::Rect::from_min_size(min, vec2(f, f));
        painter.rect_stroke(
            r,
            2,
            Stroke::new(size * 0.05, color),
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(r.shrink(f * 0.32), 1, color);
    }
    let dot = egui::Rect::from_min_size(
        rect.max - vec2(m + f * 0.7, m + f * 0.7),
        vec2(f * 0.7, f * 0.7),
    );
    painter.rect_filled(dot, 2, color.gamma_multiply(0.85));
}

/// Vector icons. Emoji fonts are not available everywhere (Android has none in egui), so we draw them.
#[derive(Clone, Copy)]
pub enum Icon {
    File,
    Folder,
    Camera,
    Qr,
    Check,
    Sun,
    Moon,
}

/// Paint `icon` into a square `rect`. `bg` is the colour behind it (the moon cuts it out).
pub fn paint_icon(
    painter: &egui::Painter,
    icon: Icon,
    rect: egui::Rect,
    color: Color32,
    bg: Color32,
) {
    let s = rect.width();
    let at = |x: f32, y: f32| rect.min + vec2(x * s, y * s);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| egui::Rect::from_min_max(at(x0, y0), at(x1, y1));
    let stroke = Stroke::new((s * 0.08).max(1.5), color);
    let round = (s * 0.1) as u8;
    match icon {
        Icon::File => {
            painter.rect_stroke(
                r(0.2, 0.08, 0.8, 0.92),
                round,
                stroke,
                egui::StrokeKind::Inside,
            );
            for y in [0.38, 0.54, 0.7] {
                painter.line_segment([at(0.34, y), at(0.66, y)], stroke);
            }
        }
        Icon::Folder => {
            painter.rect_filled(r(0.08, 0.18, 0.46, 0.34), round, color);
            painter.rect_filled(r(0.08, 0.28, 0.92, 0.82), round, color);
        }
        Icon::Camera => {
            painter.rect_filled(r(0.34, 0.18, 0.66, 0.32), round, color);
            painter.rect_stroke(
                r(0.08, 0.28, 0.92, 0.84),
                round,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(at(0.5, 0.56), s * 0.15, stroke);
        }
        Icon::Qr => finders(painter, rect, color),
        Icon::Check => {
            painter.line(
                vec![at(0.18, 0.52), at(0.4, 0.74), at(0.82, 0.28)],
                Stroke::new(s * 0.1, color),
            );
        }
        Icon::Sun => {
            let c = rect.center();
            painter.circle_filled(c, s * 0.2, color);
            for k in 0..8 {
                let d = egui::Vec2::angled(k as f32 * std::f32::consts::TAU / 8.0);
                painter.line_segment(
                    [c + d * s * 0.32, c + d * s * 0.46],
                    Stroke::new(s * 0.08, color),
                );
            }
        }
        Icon::Moon => {
            let c = rect.center();
            painter.circle_filled(c, s * 0.36, color);
            painter.circle_filled(c + vec2(s * 0.18, -s * 0.14), s * 0.3, bg);
        }
    }
}
