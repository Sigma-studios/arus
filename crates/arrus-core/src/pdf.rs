//! PDF export of a choreography: one A4 landscape page per formation, with
//! the formation's name on top and the stage drawn as in the editor (grid,
//! backstage, stage directions, props, dancers with their labels and names)
//! and its comments below. For people who do not use the app.
//!
//! A small hand-written PDF 1.4 writer: vector drawing and the standard
//! Helvetica fonts (no embedding; text is encoded as WinAnsi, which covers
//! Western European names, others become `?`).

use std::fmt::Write as _;

use crate::layout::{self, Area};
use crate::{Argb, Choreography, DancerFillStyle, Formation, PropType, timeline};

/// A4 landscape, in points.
const PAGE_W: f64 = 842.0;
const PAGE_H: f64 = 595.0;
const MARGIN: f64 = 36.0;
/// Room around the stage for AUDIENCE / BACKSTAGE / STAGE LEFT / RIGHT.
const LABEL_ROOM: f64 = 18.0;

/// The whole PDF file.
pub fn render(c: &Choreography) -> Vec<u8> {
    let n = c.formations.len();
    let pages: Vec<Vec<u8>> = if n == 0 {
        let mut p = Canvas::page();
        p.text(Font::Bold, 22.0, MARGIN, PAGE_H - MARGIN - 18.0, &c.name, Align::Left, TEXT);
        p.text(Font::Regular, 11.0, MARGIN, PAGE_H - MARGIN - 40.0, "No formations yet.", Align::Left, TEXT_WEAK);
        vec![p.buf]
    } else {
        (0..n).map(|f| page(c, f)).collect()
    };
    write_file(&c.name, &pages)
}

// ---- one page --------------------------------------------------------------

type Rgb = (f64, f64, f64);
// A dark page, easy on the eyes on screen (matches the app's theme).
const PAGE_BG: Rgb = (0.075, 0.075, 0.085);
const TEXT: Rgb = (0.92, 0.92, 0.94);
const TEXT_WEAK: Rgb = (0.66, 0.66, 0.70);
const TEXT_FAINT: Rgb = (0.48, 0.48, 0.53);
/// Near-black: text on light dancers, and their outlines.
const INK: Rgb = (0.07, 0.07, 0.08);
const ACCENT: Rgb = (0.66, 0.50, 0.98);
const BACKSTAGE: Rgb = (0.115, 0.115, 0.13);
const STAGE_FLOOR: Rgb = (0.165, 0.165, 0.185);
const GRID_LINE: Rgb = (0.26, 0.26, 0.29);
const BACKSTAGE_EDGE: Rgb = (0.34, 0.34, 0.38);
const STAGE_EDGE: Rgb = (0.64, 0.64, 0.69);

fn rgb_of(argb: i64) -> Rgb {
    let a = Argb::from_i64(argb);
    (f64::from(a.r) / 255.0, f64::from(a.g) / 255.0, f64::from(a.b) / 255.0)
}

/// Black or white, whichever reads better on `c`.
fn on_color(c: Rgb) -> Rgb {
    if 0.299 * c.0 + 0.587 * c.1 + 0.114 * c.2 > 0.6 { INK } else { (1.0, 1.0, 1.0) }
}

fn mix(c: Rgb, with: Rgb, t: f64) -> Rgb {
    (c.0 + (with.0 - c.0) * t, c.1 + (with.1 - c.1) * t, c.2 + (with.2 - c.2) * t)
}

fn fmt_secs(s: f64) -> String {
    if (s - s.round()).abs() < 1e-6 { format!("{s:.0} s") } else { format!("{s:.1} s") }
}

fn fmt_clock(s: f64) -> String {
    let s = s.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The formation's title: its name, or "Formation 3".
pub fn formation_title(f: &Formation, index: usize) -> String {
    match f.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_owned(),
        None => format!("Formation {}", index + 1),
    }
}

/// Maps grid cells to page points, oriented as in the editor: the audience
/// at the top unless `isSceneDirectionsInverted` (backstage at the top).
#[derive(Debug, Clone, Copy)]
struct Frame {
    b: Area,
    s: f64,
    ox: f64,
    oy: f64,
    backstage_top: bool,
}

impl Frame {
    fn fit(b: Area, x: f64, y: f64, w: f64, h: f64, backstage_top: bool) -> Self {
        let s = (w / b.width().max(0.1)).min(h / b.height().max(0.1));
        let ox = x + (w - b.width() * s) / 2.0;
        let oy = y + (h - b.height() * s) / 2.0;
        Self { b, s, ox, oy, backstage_top }
    }

    /// PDF coordinates (y up).
    fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let (b, s) = (&self.b, self.s);
        if self.backstage_top {
            (self.ox + (x - b.x0) * s, self.oy + (b.y1 - y) * s)
        } else {
            (self.ox + (b.x1 - x) * s, self.oy + (y - b.y0) * s)
        }
    }

    /// An area as (x, y, w, h) in points.
    fn rect(&self, a: Area) -> (f64, f64, f64, f64) {
        let (x0, y0) = self.map(a.x0, a.y0);
        let (x1, y1) = self.map(a.x1, a.y1);
        (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
    }
}

fn page(c: &Choreography, f: usize) -> Vec<u8> {
    let fm = &c.formations[f];
    let n = c.formations.len();
    let set = &c.settings;
    let mut p = Canvas::page();

    // Header: the formation's name, then where it sits in the show.
    let top = PAGE_H - MARGIN;
    let title = formation_title(fm, f);
    let right_text = fit_text(&c.name, Font::Regular, 10.0, 260.0);
    let title_w = PAGE_W - 2.0 * MARGIN - text_width(&right_text, Font::Regular, 10.0) - 24.0;
    p.text(Font::Bold, 22.0, MARGIN, top - 18.0, &fit_text(&title, Font::Bold, 22.0, title_w), Align::Left, TEXT);
    p.text(Font::Regular, 10.0, PAGE_W - MARGIN, top - 14.0, &right_text, Align::Right, TEXT_WEAK);
    let mut sub = format!("Formation {} of {n} \u{b7} starts at {}", f + 1, fmt_clock(timeline::formation_time(c, f)));
    sub.push_str(&format!(" \u{b7} hold {}", fmt_secs(fm.duration)));
    if f > 0 {
        sub.push_str(&format!(" \u{b7} transition {}", fmt_secs(fm.transition_duration)));
    }
    p.text(Font::Regular, 10.5, MARGIN, top - 36.0, &sub, Align::Left, TEXT_WEAK);
    p.stroke_color(mix(ACCENT, PAGE_BG, 0.45));
    p.line_width(0.8);
    p.line(MARGIN, top - 46.0, PAGE_W - MARGIN, top - 46.0);

    // Comments at the bottom (a few lines at most).
    let comments: Vec<&str> = fm.comments.iter().map(|c| c.text.trim()).filter(|t| !t.is_empty()).collect();
    let mut lines = Vec::new();
    for t in &comments {
        for para in t.lines() {
            lines.extend(wrap(para, Font::Regular, 10.0, PAGE_W - 2.0 * MARGIN - 70.0));
        }
    }
    if lines.len() > 4 {
        lines.truncate(4);
        if let Some(l) = lines.last_mut() {
            l.push_str(" \u{2026}");
        }
    }
    let footer = MARGIN - 14.0;
    let comments_h = if lines.is_empty() { 0.0 } else { 14.0 * lines.len() as f64 + 10.0 };
    for (k, l) in lines.iter().enumerate() {
        let y = footer + 16.0 + 14.0 * (lines.len() - 1 - k) as f64;
        if k == 0 {
            p.text(Font::Bold, 10.0, MARGIN, y, "Notes", Align::Left, ACCENT);
        }
        p.text(Font::Regular, 10.0, MARGIN + 70.0, y, l, Align::Left, TEXT);
    }
    p.text(Font::Regular, 8.0, PAGE_W - MARGIN, footer, &format!("Page {} of {n}", f + 1), Align::Right, TEXT_FAINT);

    // The stage.
    let grid = layout::grid_area(c);
    let stage = layout::stage_area(c);
    let radius = set.dancer_size_multiplier.clamp(0.1, 3.0) / 2.0;
    let mut b = grid;
    for u in &fm.units {
        b.x0 = b.x0.min(u.position.x - radius);
        b.y0 = b.y0.min(u.position.y - radius);
        b.x1 = b.x1.max(u.position.x + radius);
        b.y1 = b.y1.max(u.position.y + radius);
    }
    let area_bottom = footer + 14.0 + comments_h;
    let area_top = top - 56.0;
    let fr = Frame::fit(
        b,
        MARGIN + LABEL_ROOM,
        area_bottom + LABEL_ROOM,
        PAGE_W - 2.0 * (MARGIN + LABEL_ROOM),
        area_top - area_bottom - 2.0 * LABEL_ROOM,
        set.is_scene_directions_inverted,
    );
    let backstage = layout::backstage_on(c);
    let (gx, gy, gw, gh) = fr.rect(grid);
    let (sx, sy, sw, sh) = fr.rect(stage);
    if backstage {
        p.fill_color(BACKSTAGE);
        p.rect(gx, gy, gw, gh, Paint::Fill);
    }
    p.fill_color(STAGE_FLOOR);
    p.rect(sx, sy, sw, sh, Paint::Fill);
    if set.is_grid_shown {
        p.stroke_color(GRID_LINE);
        p.line_width(0.4);
        let mut x = grid.x0 + 1.0;
        while x < grid.x1 - 1e-6 {
            let (a, b2) = (fr.map(x, grid.y0), fr.map(x, grid.y1));
            p.line(a.0, a.1, b2.0, b2.1);
            x += 1.0;
        }
        let mut y = grid.y0 + 1.0;
        while y < grid.y1 - 1e-6 {
            let (a, b2) = (fr.map(grid.x0, y), fr.map(grid.x1, y));
            p.line(a.0, a.1, b2.0, b2.1);
            y += 1.0;
        }
    }
    // Centre line.
    let mid = (stage.x0 + stage.x1) / 2.0;
    let (a, b2) = (fr.map(mid, stage.y0), fr.map(mid, stage.y1));
    p.stroke_color(TEXT_FAINT);
    p.line_width(0.8);
    p.dash(4.0, 3.0);
    p.line(a.0, a.1, b2.0, b2.1);
    p.undash();
    if backstage {
        p.stroke_color(BACKSTAGE_EDGE);
        p.line_width(0.6);
        p.rect(gx, gy, gw, gh, Paint::Stroke);
    }
    p.stroke_color(STAGE_EDGE);
    p.line_width(1.2);
    p.rect(sx, sy, sw, sh, Paint::Stroke);

    if set.is_scene_directions_shown {
        let center = fr.map((grid.x0 + grid.x1) / 2.0, (grid.y0 + grid.y1) / 2.0);
        let outer = Area { x0: b.x0.min(grid.x0), y0: b.y0.min(grid.y0), x1: b.x1.max(grid.x1), y1: b.y1.max(grid.y1) };
        let gmid = (outer.x0 + outer.x1) / 2.0;
        let ymid = (outer.y0 + outer.y1) / 2.0;
        let place = |x: f64, y: f64, d: f64| {
            let e = fr.map(x, y);
            let (dx, dy) = (e.0 - center.0, e.1 - center.1);
            let l = dx.hypot(dy).max(1e-6);
            (e.0 + dx / l * d, e.1 + dy / l * d)
        };
        let aud = place(gmid, outer.y1, 11.0);
        let back = place(gmid, outer.y0, 11.0);
        p.text(Font::Bold, 9.0, aud.0, aud.1 - 3.0, "AUDIENCE", Align::Center, TEXT_WEAK);
        p.text(Font::Regular, 9.0, back.0, back.1 - 3.0, "BACKSTAGE", Align::Center, TEXT_FAINT);
        for (x, label) in [(outer.x1, "STAGE RIGHT"), (outer.x0, "STAGE LEFT")] {
            let at = place(x, ymid, 10.0);
            // Reads bottom-to-top on the left of the page, top-to-bottom on the right.
            let angle = if at.0 < center.0 { 90.0 } else { -90.0 };
            p.text_rotated(Font::Regular, 9.0, at.0, at.1, angle, label, TEXT_FAINT);
        }
    }

    // Props.
    for prop in fm.props.as_deref().unwrap_or(&[]) {
        let (hx, hy) = (prop.width.abs() / 2.0, prop.length.abs() / 2.0);
        let local: Vec<(f64, f64)> = match prop.kind() {
            PropType::Rectangle => vec![(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)],
            PropType::Triangle => vec![(0.0, -hy), (hx, hy), (-hx, hy)],
            PropType::Ellipse => (0..36)
                .map(|k| {
                    let t = f64::from(k) / 36.0 * std::f64::consts::TAU;
                    (hx * t.cos(), hy * t.sin())
                })
                .collect(),
        };
        let (sn, cs) = prop.rotation_angle.sin_cos();
        let pts: Vec<(f64, f64)> = local
            .iter()
            .map(|&(x, y)| fr.map(prop.position.x + x * cs - y * sn, prop.position.y + x * sn + y * cs))
            .collect();
        let col = rgb_of(prop.color);
        p.fill_color(mix(col, STAGE_FLOOR, 1.0 - prop.alpha.clamp(0.1, 1.0) * 0.8));
        p.stroke_color(mix(col, TEXT, 0.25));
        p.line_width(0.6);
        p.polygon(&pts, Paint::FillStroke);
    }

    // Dancers, then their names on top.
    let rp = radius * fr.s;
    let label_mult = set.dancer_label_size_multiplier.clamp(0.3, 3.0);
    let fill_style = set.fill_style();
    for (i, u) in fm.units.iter().enumerate() {
        let (cx, cy) = fr.map(u.position.x, u.position.y);
        let col = rgb_of(u.color);
        p.fill_color(col);
        p.stroke_color(mix(col, INK, 0.5));
        p.line_width(0.6);
        match layout::shape_outline(u.shape()) {
            None => p.circle(cx, cy, rp, Paint::FillStroke),
            Some(pts) => {
                let pts: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| (cx + x * rp, cy - y * rp)).collect();
                p.polygon(&pts, Paint::FillStroke);
            }
        }
        let label = match fill_style {
            DancerFillStyle::Indexed => Some((i + 1).to_string()),
            DancerFillStyle::ShortNamed => u.short_name.clone().filter(|s| !s.trim().is_empty()),
            DancerFillStyle::Solid => None,
        };
        if let Some(label) = label {
            let chars = label.chars().count().max(1) as f64;
            let size = (rp * if chars <= 2.0 { 0.95 } else { 1.9 / chars }).clamp(3.5, 40.0) * label_mult;
            p.text(Font::Bold, size, cx, cy - size * 0.35, &label, Align::Center, on_color(col));
        }
    }
    if set.is_dancer_names_shown {
        for u in &fm.units {
            let name = u.dancer.name.trim();
            if name.is_empty() {
                continue;
            }
            let (cx, cy) = fr.map(u.position.x, u.position.y);
            let size = (rp * 0.62).clamp(5.0, 11.0) * label_mult;
            p.text(Font::Regular, size, cx, cy - rp - size * 0.95, name, Align::Center, TEXT);
        }
    }
    p.buf
}

// ---- drawing ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Font {
    Regular,
    Bold,
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
enum Paint {
    Fill,
    Stroke,
    FillStroke,
}

impl Paint {
    fn op(self) -> &'static str {
        match self {
            Paint::Fill => "f",
            Paint::Stroke => "S",
            Paint::FillStroke => "B",
        }
    }
}

/// A page's content stream.
#[derive(Default)]
struct Canvas {
    buf: Vec<u8>,
}

fn n(v: f64) -> String {
    let s = format!("{:.2}", if v.is_finite() { v } else { 0.0 });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_owned() }
}

impl Canvas {
    /// A new page, already painted with the dark background.
    fn page() -> Self {
        let mut p = Self::default();
        p.fill_color(PAGE_BG);
        p.rect(0.0, 0.0, PAGE_W, PAGE_H, Paint::Fill);
        p
    }
    fn op(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(b'\n');
    }
    fn fill_color(&mut self, c: Rgb) {
        self.op(&format!("{} {} {} rg", n(c.0), n(c.1), n(c.2)));
    }
    fn stroke_color(&mut self, c: Rgb) {
        self.op(&format!("{} {} {} RG", n(c.0), n(c.1), n(c.2)));
    }
    fn line_width(&mut self, w: f64) {
        self.op(&format!("{} w", n(w)));
    }
    fn dash(&mut self, on: f64, off: f64) {
        self.op(&format!("[{} {}] 0 d", n(on), n(off)));
    }
    fn undash(&mut self) {
        self.op("[] 0 d");
    }
    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        self.op(&format!("{} {} m {} {} l S", n(x0), n(y0), n(x1), n(y1)));
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, paint: Paint) {
        self.op(&format!("{} {} {} {} re {}", n(x), n(y), n(w), n(h), paint.op()));
    }
    fn polygon(&mut self, pts: &[(f64, f64)], paint: Paint) {
        let Some((first, rest)) = pts.split_first() else { return };
        let mut s = format!("{} {} m", n(first.0), n(first.1));
        for p in rest {
            let _ = write!(s, " {} {} l", n(p.0), n(p.1));
        }
        let _ = write!(s, " h {}", paint.op());
        self.op(&s);
    }
    fn circle(&mut self, cx: f64, cy: f64, r: f64, paint: Paint) {
        // Four Bézier quarter arcs.
        let k = 0.552_284_75 * r;
        self.op(&format!(
            "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h {}",
            n(cx + r), n(cy),
            n(cx + r), n(cy + k), n(cx + k), n(cy + r), n(cx), n(cy + r),
            n(cx - k), n(cy + r), n(cx - r), n(cy + k), n(cx - r), n(cy),
            n(cx - r), n(cy - k), n(cx - k), n(cy - r), n(cx), n(cy - r),
            n(cx + k), n(cy - r), n(cx + r), n(cy - k), n(cx + r), n(cy),
            paint.op()
        ));
    }
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, font: Font, size: f64, x: f64, y: f64, s: &str, align: Align, color: Rgb) {
        let w = text_width(s, font, size);
        let x = match align {
            Align::Left => x,
            Align::Center => x - w / 2.0,
            Align::Right => x - w,
        };
        self.fill_color(color);
        self.op(&format!("BT /{} {} Tf {} {} Td", font_res(font), n(size), n(x), n(y)));
        self.show(s);
        self.op("ET");
    }
    /// Text centred at (x, y), turned by `angle` degrees (counter-clockwise).
    #[allow(clippy::too_many_arguments)]
    fn text_rotated(&mut self, font: Font, size: f64, x: f64, y: f64, angle: f64, s: &str, color: Rgb) {
        let w = text_width(s, font, size);
        let (sn, cs) = angle.to_radians().sin_cos();
        // Start of the baseline, so the text's centre lands on (x, y).
        let (dx, dy) = (-w / 2.0, -size * 0.35);
        let (tx, ty) = (x + dx * cs - dy * sn, y + dx * sn + dy * cs);
        self.fill_color(color);
        self.op(&format!(
            "BT /{} {} Tf {} {} {} {} {} {} Tm",
            font_res(font),
            n(size),
            n(cs),
            n(sn),
            n(-sn),
            n(cs),
            n(tx),
            n(ty)
        ));
        self.show(s);
        self.op("ET");
    }
    fn show(&mut self, s: &str) {
        self.buf.push(b'(');
        self.buf.extend(encode(s));
        self.buf.extend_from_slice(b") Tj\n");
    }
}

fn font_res(f: Font) -> &'static str {
    match f {
        Font::Regular => "F1",
        Font::Bold => "F2",
    }
}

// ---- text ------------------------------------------------------------------

/// WinAnsi byte for a character, if it has one.
fn win_ansi(c: char) -> Option<u8> {
    let b = match c {
        ' '..='~' => c as u8,
        '\u{a0}'..='\u{ff}' => c as u32 as u8,
        '\u{20ac}' => 0x80,
        '\u{201a}' => 0x82,
        '\u{201e}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{0160}' => 0x8A,
        '\u{0152}' => 0x8C,
        '\u{017d}' => 0x8E,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{0161}' => 0x9A,
        '\u{0153}' => 0x9C,
        '\u{017e}' => 0x9E,
        '\u{0178}' => 0x9F,
        _ => return None,
    };
    Some(b)
}

/// A PDF string body (without the parentheses).
fn encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        match win_ansi(c) {
            Some(b @ (b'(' | b')' | b'\\')) => out.extend_from_slice(&[b'\\', b]),
            Some(b) => out.push(b),
            None if c.is_whitespace() => out.push(b' '),
            None => out.push(b'?'),
        }
    }
    out
}

/// Helvetica advance widths (1/1000 em) for ' '..='~'.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015, // ':'..'@'
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667,
    944, 667, 667, 611, // A-Z
    278, 278, 278, 469, 556, 333, // '['..'`'
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500,
    722, 500, 500, 500, // a-z
    334, 260, 334, 584, // '{'..'~'
];

fn text_width(s: &str, font: Font, size: f64) -> f64 {
    let units: f64 = s
        .chars()
        .map(|c| match c {
            ' '..='~' => f64::from(HELVETICA[c as usize - 32]),
            _ => 556.0,
        })
        .sum();
    // Bold runs about 6 % wider.
    units / 1000.0 * size * if font == Font::Bold { 1.06 } else { 1.0 }
}

/// `s`, shortened with "…" to fit `max` points.
fn fit_text(s: &str, font: Font, size: f64, max: f64) -> String {
    if text_width(s, font, size) <= max {
        return s.to_owned();
    }
    let mut out: String = s.to_owned();
    while !out.is_empty() && text_width(&format!("{out}\u{2026}"), font, size) > max {
        out.pop();
    }
    format!("{}\u{2026}", out.trim_end())
}

/// Break `s` into lines of at most `max` points.
fn wrap(s: &str, font: Font, size: f64, max: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
        if text_width(&candidate, font, size) <= max || cur.is_empty() {
            cur = candidate;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_owned();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.into_iter().map(|l| fit_text(&l, font, size, max)).collect()
}

// ---- file ------------------------------------------------------------------

fn write_file(title: &str, pages: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        let id = offsets.len();
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // 1 catalog, 2 page tree, 3-4 fonts, 5 info, then a page + its content per page.
    let first_page = 6;
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", first_page + 2 * i)).collect();
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes());
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>");
    let mut info = b"<< /Producer (arus) /Title (".to_vec();
    info.extend(encode(title));
    info.extend_from_slice(b") >>");
    obj(&mut out, &info);
    for (i, content) in pages.iter().enumerate() {
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] \
             /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
            first_page + 2 * i + 1
        );
        obj(&mut out, page.as_bytes());
        let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        stream.extend_from_slice(content);
        stream.extend_from_slice(b"\nendstream");
        obj(&mut out, &stream);
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1);
    out.extend_from_slice(tail.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Comment, Unit};

    fn sample() -> Choreography {
        let mut c = Choreography::new("Show (2026)", "id", 0);
        c.formations[0].name = Some("Intro".into());
        c.add_dancer(Unit::at(2.0, 3.0));
        c.add_dancer(Unit::at(-1.0, 4.0)); // outside the grid
        c.formations[0].units[0].dancer.name = "Jürgen".into();
        let mut second = c.formations[0].clone();
        second.name = None;
        second.comments.push(Comment { text: "Everyone faces the audience. ".repeat(20), extra: Default::default() });
        c.formations.push(second);
        c
    }

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn one_page_per_formation_and_a_valid_xref() {
        let c = sample();
        let pdf = render(&c);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let pages = pdf.windows(b"/Type /Page ".len()).filter(|w| w == b"/Type /Page ").count();
        assert_eq!(pages, 2);
        // Every xref entry points at its object.
        let xref = find(&pdf, b"\nxref\n").unwrap() + 1;
        let table = String::from_utf8_lossy(&pdf[xref..]).into_owned();
        for (id, line) in table.lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", id + 1).as_bytes()), "object {}", id + 1);
        }
        let start: usize = table.lines().rev().nth(1).unwrap().parse().unwrap();
        assert_eq!(start, xref);
        // Titles: the name, or "Formation 2"; WinAnsi umlauts; escaped parentheses.
        assert!(find(&pdf, b"(Intro) Tj").is_some());
        assert!(find(&pdf, b"(Formation 2) Tj").is_some());
        assert!(find(&pdf, b"(J\xFCrgen) Tj").is_some());
        assert!(find(&pdf, b"Show \\(2026\\)").is_some());
        assert!(find(&pdf, b"(Notes) Tj").is_some());
    }

    #[test]
    fn no_formations_still_makes_a_page() {
        let mut c = Choreography::new("Empty", "id", 0);
        c.formations.clear();
        let pdf = render(&c);
        assert!(find(&pdf, b"/Count 1").is_some());
    }

    #[test]
    fn orientation_follows_the_editor() {
        let b = Area { x0: 0.0, y0: 0.0, x1: 10.0, y1: 5.0 };
        // Default: audience (y = 5) at the top, stage right (x = 10) on the left.
        let fr = Frame::fit(b, 0.0, 0.0, 100.0, 50.0, false);
        assert!(fr.map(5.0, 5.0).1 > fr.map(5.0, 0.0).1);
        assert!(fr.map(10.0, 2.0).0 < fr.map(0.0, 2.0).0);
        // Inverted: backstage (y = 0) at the top.
        let fr = Frame::fit(b, 0.0, 0.0, 100.0, 50.0, true);
        assert!(fr.map(5.0, 0.0).1 > fr.map(5.0, 5.0).1);
        assert_eq!(fr.rect(b), (0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn text_helpers() {
        assert_eq!(encode("a(b)\\c"), b"a\\(b\\)\\\\c".to_vec());
        assert_eq!(encode("ü – 漢"), vec![0xFC, b' ', 0x96, b' ', b'?']);
        assert_eq!(text_width("ii", Font::Regular, 10.0), 4.44);
        let short = fit_text(&"W".repeat(100), Font::Regular, 10.0, 50.0);
        assert!(text_width(&short, Font::Regular, 10.0) <= 50.0 && short.ends_with('\u{2026}'));
        let lines = wrap("one two three four five six seven", Font::Regular, 10.0, 60.0);
        assert!(lines.len() > 1 && lines.iter().all(|l| text_width(l, Font::Regular, 10.0) <= 60.0));
        assert_eq!(n(1.0), "1");
        assert_eq!(n(-0.001), "0");
        assert_eq!(n(2.5), "2.5");
    }
}
