//! The dark theme: one palette shared by widgets and the stage painter.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, Vec2};

use std::cell::Cell;

pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE8, 0xEB);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA8);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x70, 0x70, 0x78);

/// Selection on the stage: a cool colour that stands apart from the accent.
pub const SELECT: Color32 = Color32::from_rgb(0x5C, 0xC8, 0xFF);

pub const OK: Color32 = Color32::from_rgb(0x2E, 0xC2, 0x7E);
pub const WARN: Color32 = Color32::from_rgb(0xF5, 0xA5, 0x24);
pub const ERROR: Color32 = Color32::from_rgb(0xF0, 0x52, 0x52);

/// The default background (the darkest surface).
pub const DEFAULT_BG: Color32 = Color32::from_rgb(0x0B, 0x0B, 0x0C);
/// Brand accent: a violet bright enough for text on the panels and dark
/// enough for white text on a primary button.
pub const DEFAULT_ACCENT: Color32 = Color32::from_rgb(0x92, 0x66, 0xF8);

/// The colours a person can change (background and accent) and everything
/// derived from them. Surfaces keep their default distance from the
/// background, so a tinted background tints the whole UI and the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub hover_edge: Color32,
    pub border: Color32,
    pub stage_floor: Color32,
    pub backstage: Color32,
    pub grid: Color32,
    pub grid_sub: Color32,
    pub grid_center: Color32,
    pub stage_edge: Color32,
    pub hold: Color32,
    pub transition: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub hold_current: Color32,
}

/// Brightest background channel allowed: the text stays light, so the
/// background has to stay dark.
pub const MAX_BG_CHANNEL: u8 = 0x30;
/// Dimmest accent allowed (luma), so it still reads as text on the panels.
const MIN_ACCENT_LUMA: f32 = 90.0;

impl Palette {
    pub fn new(bg: Color32, accent: Color32) -> Self {
        let bg = clamp_background(bg);
        let accent = clamp_accent(accent);
        // Each surface sits where it does on the default background.
        let at = |r: u8, g: u8, b: u8| {
            let ch = |base: u8, def_base: u8, v: u8| (base as i16 + v as i16 - def_base as i16).clamp(0, 255) as u8;
            let d = DEFAULT_BG;
            Color32::from_rgb(ch(bg.r(), d.r(), r), ch(bg.g(), d.g(), g), ch(bg.b(), d.b(), b))
        };
        Self {
            bg,
            panel: at(0x13, 0x13, 0x15),
            surface: at(0x1A, 0x1A, 0x1D),
            raised: at(0x24, 0x24, 0x28),
            hover: at(0x2E, 0x2E, 0x33),
            hover_edge: at(0x48, 0x48, 0x50),
            border: at(0x2C, 0x2C, 0x31),
            stage_floor: at(0x20, 0x20, 0x23),
            backstage: at(0x16, 0x16, 0x18),
            grid: at(0x3A, 0x3A, 0x40),
            grid_sub: at(0x2A, 0x2A, 0x2F),
            grid_center: at(0x58, 0x58, 0x61),
            stage_edge: at(0x74, 0x74, 0x7E),
            hold: at(0x2B, 0x2B, 0x30),
            transition: at(0x19, 0x19, 0x1C),
            accent,
            accent_dim: lerp(bg, accent, 0.35),
            hold_current: lerp(bg, accent, 0.33),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new(DEFAULT_BG, DEFAULT_ACCENT)
    }
}

/// Scale a background down until its brightest channel is at most [`MAX_BG_CHANNEL`].
pub fn clamp_background(c: Color32) -> Color32 {
    let max = c.r().max(c.g()).max(c.b());
    if max <= MAX_BG_CHANNEL {
        return c;
    }
    let f = MAX_BG_CHANNEL as f32 / max as f32;
    let s = |v: u8| (v as f32 * f).round() as u8;
    Color32::from_rgb(s(c.r()), s(c.g()), s(c.b()))
}

/// Lighten an accent towards white until it reads on the dark panels.
pub fn clamp_accent(c: Color32) -> Color32 {
    let mut c = Color32::from_rgb(c.r(), c.g(), c.b());
    for _ in 0..20 {
        if luma(c) >= MIN_ACCENT_LUMA {
            break;
        }
        c = lerp(c, Color32::WHITE, 0.1);
    }
    c
}

fn luma(c: Color32) -> f32 {
    0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

thread_local! {
    static PALETTE: Cell<Palette> = Cell::new(Palette::default());
    static GENERATION: Cell<u32> = const { Cell::new(0) };
}

/// The palette in use.
pub fn palette() -> Palette {
    PALETTE.with(Cell::get)
}

/// Switch palette; [`generation`] changes so the widget style is rebuilt.
pub fn set_palette(p: Palette) {
    if palette() != p {
        PALETTE.with(|c| c.set(p));
        GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    }
}

/// Changes on every [`set_palette`] that changed something.
pub fn generation() -> u32 {
    GENERATION.with(Cell::get)
}

macro_rules! palette_colors {
    ($($(#[$m:meta])* $f:ident),+ $(,)?) => {
        $($(#[$m])* pub fn $f() -> Color32 { palette().$f })+
    };
}

palette_colors!(
    bg, panel, surface, raised, hover, border,
    // Stage
    stage_floor, backstage, grid, grid_sub, grid_center, stage_edge,
    // Timeline
    hold, transition, hold_current,
    accent, accent_dim,
);

/// Install the theme. `touch` enlarges hit targets and text for phones.
pub fn apply(ctx: &egui::Context, touch: bool) {
    let p = palette();
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
        v.panel_fill = p.panel;
        v.window_fill = p.surface;
        v.window_stroke = Stroke::new(1.0, p.border);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = p.bg;
        v.faint_bg_color = p.surface;
        v.code_bg_color = p.bg;
        v.hyperlink_color = SELECT;
        v.warn_fg_color = WARN;
        v.error_fg_color = ERROR;
        v.selection.bg_fill = p.accent_dim;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.striped = true;

        let r = CornerRadius::same(6);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.panel;
        w.noninteractive.weak_bg_fill = p.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = p.raised;
        w.inactive.weak_bg_fill = p.raised;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = p.hover;
        w.hovered.weak_bg_fill = p.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, p.hover_edge);
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = p.accent_dim;
        w.active.weak_bg_fill = p.accent_dim;
        w.active.bg_stroke = Stroke::new(1.0, p.accent);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = p.raised;
        w.open.weak_bg_fill = p.raised;
        w.open.bg_stroke = Stroke::new(1.0, p.border);
        w.open.corner_radius = r;
    });
}

/// A filled "primary" button colour.
pub fn primary_fill() -> Color32 {
    accent()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_keeps_the_original_colours() {
        let p = Palette::default();
        assert_eq!(p.bg, Color32::from_rgb(0x0B, 0x0B, 0x0C));
        assert_eq!(p.panel, Color32::from_rgb(0x13, 0x13, 0x15));
        assert_eq!(p.border, Color32::from_rgb(0x2C, 0x2C, 0x31));
        assert_eq!(p.stage_edge, Color32::from_rgb(0x74, 0x74, 0x7E));
        assert_eq!(p.accent, DEFAULT_ACCENT);
        // White text stays on the default accent (primary buttons).
        assert_eq!(on_color(p.accent), Color32::WHITE);
    }

    #[test]
    fn surfaces_follow_the_background() {
        let p = Palette::new(Color32::from_rgb(0x0A, 0x10, 0x1C), DEFAULT_ACCENT);
        // Same distance from the background as by default, per channel.
        assert_eq!(p.panel, Color32::from_rgb(0x12, 0x18, 0x25));
    }

    #[test]
    fn colours_stay_readable() {
        assert_eq!(clamp_background(Color32::WHITE), Color32::from_rgb(MAX_BG_CHANNEL, MAX_BG_CHANNEL, MAX_BG_CHANNEL));
        assert_eq!(clamp_background(Color32::from_rgb(0x60, 0x30, 0x00)), Color32::from_rgb(0x30, 0x18, 0x00));
        assert!(luma(clamp_accent(Color32::from_rgb(0x10, 0x00, 0x40))) >= MIN_ACCENT_LUMA);
        assert_eq!(clamp_accent(DEFAULT_ACCENT), DEFAULT_ACCENT);
        // A light accent gets dark text on buttons.
        assert_ne!(on_color(Color32::from_rgb(0xF5, 0xE0, 0x60)), Color32::WHITE);
    }
}
