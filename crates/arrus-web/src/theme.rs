//! The dark theme: one palette shared by widgets and the stage painter.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, Vec2};

pub const BG: Color32 = Color32::from_rgb(0x0B, 0x0B, 0x0C);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x13, 0x15);
pub const SURFACE: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1D);
pub const RAISED: Color32 = Color32::from_rgb(0x24, 0x24, 0x28);
pub const HOVER: Color32 = Color32::from_rgb(0x2E, 0x2E, 0x33);
pub const BORDER: Color32 = Color32::from_rgb(0x2C, 0x2C, 0x31);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE8, 0xEB);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA8);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x70, 0x70, 0x78);

/// Brand accent: a violet bright enough for text on the panels and dark
/// enough for white text on a primary button.
pub const ACCENT: Color32 = Color32::from_rgb(0x92, 0x66, 0xF8);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x3A, 0x2B, 0x63);
/// Selection on the stage: a cool colour that stands apart from the accent.
pub const SELECT: Color32 = Color32::from_rgb(0x5C, 0xC8, 0xFF);

pub const OK: Color32 = Color32::from_rgb(0x2E, 0xC2, 0x7E);
pub const WARN: Color32 = Color32::from_rgb(0xF5, 0xA5, 0x24);
pub const ERROR: Color32 = Color32::from_rgb(0xF0, 0x52, 0x52);

// Stage
pub const STAGE_FLOOR: Color32 = Color32::from_rgb(0x20, 0x20, 0x23);
pub const BACKSTAGE: Color32 = Color32::from_rgb(0x16, 0x16, 0x18);
pub const GRID: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x40);
pub const GRID_SUB: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2F);
pub const GRID_CENTER: Color32 = Color32::from_rgb(0x58, 0x58, 0x61);
pub const STAGE_EDGE: Color32 = Color32::from_rgb(0x74, 0x74, 0x7E);

// Timeline
pub const HOLD: Color32 = Color32::from_rgb(0x2B, 0x2B, 0x30);
pub const HOLD_CURRENT: Color32 = Color32::from_rgb(0x3A, 0x2C, 0x5E);
pub const TRANSITION: Color32 = Color32::from_rgb(0x19, 0x19, 0x1C);

/// Install the theme. `touch` enlarges hit targets and text for phones.
pub fn apply(ctx: &egui::Context, touch: bool) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        let base = if touch { 15.5 } else { 14.0 };
        style.text_styles = [
            (TextStyle::Small, FontId::new(base - 2.5, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(base, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(base, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(base + 6.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(base - 0.5, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, if touch { 8.0 } else { 6.0 });
        s.button_padding = if touch { Vec2::new(14.0, 9.0) } else { Vec2::new(10.0, 4.0) };
        s.interact_size = if touch { Vec2::new(48.0, 40.0) } else { Vec2::new(40.0, 24.0) };
        s.window_margin = Margin::same(16);
        s.menu_margin = Margin::same(8);
        s.indent = 16.0;
        s.combo_width = 160.0;
        s.text_edit_width = 220.0;
        s.slider_width = 150.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = PANEL;
        v.window_fill = SURFACE;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = BG;
        v.faint_bg_color = SURFACE;
        v.code_bg_color = BG;
        v.hyperlink_color = SELECT;
        v.warn_fg_color = WARN;
        v.error_fg_color = ERROR;
        v.selection.bg_fill = ACCENT_DIM;
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.striped = true;

        let r = CornerRadius::same(6);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = RAISED;
        w.inactive.weak_bg_fill = RAISED;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = HOVER;
        w.hovered.weak_bg_fill = HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x48, 0x48, 0x50));
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = ACCENT_DIM;
        w.active.weak_bg_fill = ACCENT_DIM;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = RAISED;
        w.open.weak_bg_fill = RAISED;
        w.open.bg_stroke = Stroke::new(1.0, BORDER);
        w.open.corner_radius = r;
    });
}

/// A filled "primary" button colour.
pub fn primary_fill() -> Color32 {
    ACCENT
}

/// `Color32` from an ARGB model colour, with an extra opacity factor.
pub fn argb(v: i64, opacity: f32) -> Color32 {
    let c = arrus_core::Argb::from_i64(v);
    let a = (c.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, a)
}

/// Black or white, whichever reads better on `c`.
pub fn on_color(c: Color32) -> Color32 {
    let l = 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32;
    if l > 150.0 { Color32::from_rgb(0x12, 0x12, 0x14) } else { Color32::WHITE }
}

/// A stable colour per person (presence initials).
pub fn person_color(name: &str) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(0xE0, 0x47, 0x9E),
        Color32::from_rgb(0x3A, 0x86, 0xFF),
        Color32::from_rgb(0x2E, 0xA8, 0x6B),
        Color32::from_rgb(0xF0, 0x8A, 0x24),
        Color32::from_rgb(0x9B, 0x5D, 0xE5),
        Color32::from_rgb(0x00, 0x9F, 0xB7),
        Color32::from_rgb(0xD9, 0x48, 0x3B),
        Color32::from_rgb(0x6D, 0x8A, 0x1F),
    ];
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    COLORS[h as usize % COLORS.len()]
}
