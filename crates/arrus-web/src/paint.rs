//! Drawing the stage with an egui [`Painter`]: floor, backstage, built-in or
//! custom backgrounds, grid, labels, props, dancers, ghosts and paths.

use arrus_core::timeline::DancerAt;
use arrus_core::{Choreography, DancerFillStyle, DefaultStageBackground, Prop, StageAspectRatio};
use egui::epaint::{Mesh, TextShape, Vertex};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureId, Vec2, pos2, vec2};

use crate::edit::Selection;
use crate::geom::{self, Bounds, P, PropFrame};
use crate::theme;
use crate::view::View;

/// A decoded custom background image.
#[derive(Debug, Clone, Copy)]
pub struct BgImage {
    pub texture: TextureId,
    pub size: Vec2,
}

pub struct Scene<'a> {
    pub c: &'a Choreography,
    pub view: &'a View,
    /// Formation whose attributes (and ghost/future neighbours) are shown.
    pub formation: usize,
    /// Dancers to draw (positions at the playhead).
    pub dancers: &'a [DancerAt],
    pub props: &'a [Prop],
    pub selection: &'a Selection,
    pub hover_dancer: Option<usize>,
    pub hover_prop: Option<usize>,
    pub background: Option<BgImage>,
    /// Draw resize/rotate handles on a selected prop.
    pub handles: bool,
    /// Hide editing-only overlays while playing.
    pub playing: bool,
    /// Outlines of the next formation (a per-screen view option).
    pub next_outlines: bool,
}

/// Screen distance between a prop's front edge and its rotation handle.
pub const ROTATION_HANDLE_OFFSET: f32 = 26.0;
/// Half size of the square corner handles.
pub const HANDLE_HALF: f32 = 5.0;

/// Handle positions of a prop on screen: 4 corners (in
/// [`geom::CORNER_SIGNS`] order), the rotation knob, and the front-edge centre.
pub fn prop_handles(p: &Prop, view: &View) -> ([Pos2; 4], Pos2, Pos2) {
    let f = PropFrame::of(p);
    let corners = f.corners().map(|c| view.to_screen(c));
    let knob = view.to_screen(f.rotation_handle(ROTATION_HANDLE_OFFSET as f64 / view.scale as f64));
    let front = view.to_screen(f.rotation_handle(0.0));
    (corners, knob, front)
}

/// Dancer radius in cells.
pub fn dancer_radius(c: &Choreography) -> f64 {
    (c.settings.dancer_size_multiplier.clamp(0.1, 3.0)) / 2.0
}

/// The cells to fit into the view: the whole grid, plus any dancer left
/// outside it (older web documents put the backstage around the grid).
pub fn fit_bounds(c: &Choreography) -> Bounds {
    let mut b = geom::drawable_bounds(c);
    for p in c.formations.iter().flat_map(|f| &f.units).map(|u| u.position) {
        b.min = P::new(b.min.x.min(p.x.floor()), b.min.y.min(p.y.floor()));
        b.max = P::new(b.max.x.max(p.x.ceil()), b.max.y.max(p.y.ceil()));
    }
    b
}

pub fn paint_stage(painter: &Painter, s: &Scene<'_>) {
    let c = s.c;
    let v = s.view;
    let stage = geom::stage_bounds(c);
    let drawable = geom::drawable_bounds(c);
    let stage_rect = v.bounds_to_screen(stage);

    // Backstage: the outer cells of the grid, the floor covers the rest.
    let backstage_on = geom::backstage_on(c);
    if backstage_on {
        let r = v.bounds_to_screen(drawable);
        painter.rect_filled(r, 4.0, theme::BACKSTAGE);
        hatch(painter, r, 0.7);
        painter.rect_stroke(r, 4.0, Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);
    }

    // Floor.
    painter.rect_filled(stage_rect, 2.0, theme::STAGE_FLOOR);

    // Background.
    if let Some(bg) = &c.stage_background {
        let alpha = bg.alpha.clamp(0.0, 1.0) as f32;
        let target = if !bg.is_backstage_separated && backstage_on { drawable } else { stage };
        let builtin = bg.file.as_ref().and_then(|f| f.standard_background_value.as_deref());
        if let Some(img) = s.background {
            paint_image(painter, v, target, img, StageAspectRatio::parse(&bg.aspect_ratio), alpha);
        } else if let Some(name) = builtin {
            paint_builtin(painter, v, target, DefaultStageBackground::parse(name), alpha);
        }
    }

    paint_grid(painter, c, v, drawable, stage);
    painter.rect_stroke(stage_rect, 2.0, Stroke::new(1.5, theme::STAGE_EDGE), StrokeKind::Middle);
    paint_edge_labels(painter, c, v, stage, drawable);

    // Props.
    for (i, p) in s.props.iter().enumerate() {
        let pts: Vec<Pos2> = geom::prop_outline(p).into_iter().map(|q| v.to_screen(q)).collect();
        let fill = theme::argb(p.color, p.alpha as f32);
        let selected = matches!(s.selection, Selection::Prop(j) if *j == i);
        let stroke = if selected {
            Stroke::new(2.0, theme::SELECT)
        } else if s.hover_prop == Some(i) {
            Stroke::new(1.5, Color32::from_white_alpha(160))
        } else {
            Stroke::new(1.0, theme::argb(p.color, 1.0).gamma_multiply(0.6))
        };
        painter.add(Shape::convex_polygon(pts, fill, stroke));
        if selected && s.handles && !s.playing {
            let (corners, knob, front) = prop_handles(p, v);
            painter.line_segment([front, knob], Stroke::new(1.5, theme::SELECT));
            painter.circle(knob, HANDLE_HALF + 1.5, theme::BG, Stroke::new(2.0, theme::SELECT));
            for c in corners {
                let r = Rect::from_center_size(c, Vec2::splat(HANDLE_HALF * 2.0));
                painter.rect_filled(r, 2.0, theme::BG);
                painter.rect_stroke(r, 2.0, Stroke::new(2.0, theme::SELECT), StrokeKind::Middle);
            }
        }
    }

    let radius = dancer_radius(c);
    let rpx = (radius * v.scale as f64) as f32;
    let f = s.formation.min(c.formations.len().saturating_sub(1));
    let set = &c.settings;
    let prev = f.checked_sub(1).and_then(|i| c.formations.get(i));
    let next = c.formations.get(f + 1);
    let current = c.formations.get(f);

    // Transition paths: previous formation -> this one.
    if set.is_transition_paths_shown
        && let (Some(a), Some(b)) = (prev, current)
    {
        for (ua, ub) in a.units.iter().zip(&b.units) {
            let (pa, pb) = (v.to_screen(ua.position), v.to_screen(ub.position));
            if (pb - pa).length() < 2.0 {
                continue;
            }
            let col = theme::argb(ub.color, 0.55);
            painter.extend(Shape::dashed_line(&[pa, pb], Stroke::new(1.5, col), 6.0, 4.0));
            let dir = (pb - pa).normalized();
            let tip = pb - dir * rpx;
            let side = dir.rot90() * 4.0;
            painter.add(Shape::convex_polygon(
                vec![tip, tip - dir * 8.0 + side, tip - dir * 8.0 - side],
                col,
                Stroke::NONE,
            ));
        }
    }

    // Ghosts (previous) and future (next) formations, faintly.
    let ghost = |units: &[arrus_core::Unit], opacity: f32| {
        for u in units {
            draw_shape(painter, v, u.position, radius, u.shape(), theme::argb(u.color, opacity), Stroke::NONE);
        }
    };
    if set.is_ghost_dancers_shown
        && let Some(a) = prev
    {
        ghost(&a.units, 0.22);
    }
    if s.next_outlines
        && let Some(n) = next
    {
        for u in &n.units {
            draw_shape(
                painter,
                v,
                u.position,
                radius,
                u.shape(),
                Color32::TRANSPARENT,
                Stroke::new(1.5, theme::argb(u.color, 0.45)),
            );
        }
    }

    // Dancers.
    let fill_style = set.fill_style();
    let label_mult = set.dancer_label_size_multiplier.clamp(0.3, 3.0) as f32;
    for d in s.dancers {
        let Some(u) = current.and_then(|f| f.units.get(d.index)).or_else(|| {
            prev.and_then(|f| f.units.get(d.index))
        }) else {
            continue;
        };
        let op = d.opacity.clamp(0.0, 1.0) as f32;
        if op <= 0.01 {
            continue;
        }
        let fill = theme::argb(u.color, op);
        let selected = s.selection.has_dancer(d.index);
        let hovered = s.hover_dancer == Some(d.index);
        let outline = Stroke::new(1.0, Color32::from_black_alpha((110.0 * op) as u8));
        if selected {
            painter.circle_stroke(v.to_screen(d.position), rpx + 4.0, Stroke::new(2.5, theme::SELECT));
        } else if hovered {
            painter.circle_stroke(v.to_screen(d.position), rpx + 3.0, Stroke::new(1.5, Color32::from_white_alpha(140)));
        }
        draw_shape(painter, v, d.position, radius, u.shape(), fill, outline);

        let center = v.to_screen(d.position);
        let label = match fill_style {
            DancerFillStyle::Indexed => Some((d.index + 1).to_string()),
            DancerFillStyle::ShortNamed => u.short_name.clone().filter(|s| !s.trim().is_empty()),
            DancerFillStyle::Solid => None,
        };
        let text_col = theme::on_color(theme::argb(u.color, 1.0)).gamma_multiply(op);
        if let Some(label) = label {
            let chars = label.chars().count().max(1) as f32;
            let size = (rpx * if chars <= 2.0 { 0.95 } else { 1.9 / chars }).clamp(5.0, 48.0) * label_mult;
            if size >= 5.0 {
                painter.text(center, Align2::CENTER_CENTER, label, FontId::proportional(size), text_col);
            }
        }
        if set.is_dancer_indexes_shown && fill_style != DancerFillStyle::Indexed {
            let size = (rpx * 0.55).clamp(8.0, 16.0) * label_mult;
            painter.text(
                center + vec2(rpx * 0.8, -rpx * 0.8),
                Align2::LEFT_BOTTOM,
                (d.index + 1).to_string(),
                FontId::proportional(size),
                theme::TEXT.gamma_multiply(op),
            );
        }
        if set.is_dancer_names_shown && !u.dancer.name.trim().is_empty() {
            let size = (rpx * 0.6).clamp(9.0, 18.0) * label_mult;
            let pos = center + vec2(0.0, rpx + 2.0);
            let galley = painter.layout_no_wrap(u.dancer.name.clone(), FontId::proportional(size), theme::TEXT);
            let r = Align2::CENTER_TOP.anchor_size(pos, galley.size()).expand2(vec2(3.0, 0.0));
            painter.rect_filled(r, 3.0, Color32::from_black_alpha((120.0 * op) as u8));
            painter.galley(r.min + vec2(3.0, 0.0), galley, theme::TEXT.gamma_multiply(op));
        }
    }
}

/// Draw one dancer shape centred at `at` (cells) with `radius` (cells).
pub fn draw_shape(
    painter: &Painter,
    v: &View,
    at: P,
    radius: f64,
    shape: arrus_core::DancerShape,
    fill: Color32,
    stroke: Stroke,
) {
    let center = v.to_screen(at);
    let rpx = (radius * v.scale as f64) as f32;
    match geom::dancer_shape_outline(shape) {
        None => {
            painter.circle(center, rpx, fill, stroke);
        }
        Some(pts) => {
            let pts = pts.into_iter().map(|p| center + v.vec_to_screen(geom::scale(p, radius))).collect();
            painter.add(Shape::convex_polygon(pts, fill, stroke));
        }
    }
}

/// A shape glyph inside a small screen rect (for shape pickers), upright.
pub fn shape_icon(painter: &Painter, rect: Rect, shape: arrus_core::DancerShape, fill: Color32) {
    let r = rect.width().min(rect.height()) * 0.38;
    let c = rect.center();
    match geom::dancer_shape_outline(shape) {
        None => {
            painter.circle_filled(c, r, fill);
        }
        Some(pts) => {
            let pts = pts.into_iter().map(|p| c + vec2(p.x as f32, p.y as f32) * r).collect();
            painter.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
        }
    }
}

fn hatch(painter: &Painter, r: Rect, strength: f32) {
    let p = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    let step = 14.0;
    let col = Color32::from_white_alpha((10.0 * strength) as u8);
    let mut x = r.left() - r.height();
    while x < r.right() {
        p.line_segment([pos2(x, r.bottom()), pos2(x + r.height(), r.top())], Stroke::new(1.0, col));
        x += step;
    }
}

/// Grid lines over the whole `grid` (backstage cells included); the centre
/// line runs through the middle of the `stage`.
fn paint_grid(painter: &Painter, c: &Choreography, v: &View, grid: Bounds, stage: Bounds) {
    let set = &c.settings;
    let line = |a: P, b: P, col: Color32, w: f32| {
        painter.line_segment([v.to_screen(a), v.to_screen(b)], Stroke::new(w, col));
    };
    let (x0, y0, x1, y1) = (grid.min.x, grid.min.y, grid.max.x, grid.max.y);
    if set.is_subdivisions_shown && v.scale >= 10.0 {
        let mut x = x0 + 0.5;
        while x < x1 {
            if (x - x0).fract() != 0.0 {
                line(P::new(x, y0), P::new(x, y1), theme::GRID_SUB, 1.0);
            }
            x += 0.5;
        }
        let mut y = y0 + 0.5;
        while y < y1 {
            if (y - y0).fract() != 0.0 {
                line(P::new(x0, y), P::new(x1, y), theme::GRID_SUB, 1.0);
            }
            y += 0.5;
        }
    }
    if set.is_grid_shown {
        for i in 1..(grid.width().ceil() as i64) {
            let x = x0 + i as f64;
            line(P::new(x, y0), P::new(x, y1), theme::GRID, 1.0);
        }
        for i in 1..(grid.height().ceil() as i64) {
            let y = y0 + i as f64;
            line(P::new(x0, y), P::new(x1, y), theme::GRID, 1.0);
        }
    }
    // Centre line, always faintly visible: the reference for distance marks.
    let mid = stage.center().x;
    line(P::new(mid, stage.min.y), P::new(mid, stage.max.y), theme::GRID_CENTER, 1.5);
}

/// Text centred at `pos`, rotated by `angle` radians (clockwise).
fn rotated_text(painter: &Painter, pos: Pos2, text: &str, font: FontId, color: Color32, angle: f32) {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let at = pos - galley.size() / 2.0;
    painter.add(TextShape::new(at, galley, color).with_angle_and_anchor(angle, Align2::CENTER_CENTER));
}

fn fmt_mark(d: f64) -> String {
    if d.fract() == 0.0 { format!("{d:.0}") } else { format!("{d:.1}") }
}

fn paint_edge_labels(painter: &Painter, c: &Choreography, v: &View, stage: Bounds, outer: Bounds) {
    let set = &c.settings;
    let mid_x = stage.center().x;
    // Unit screen directions of stage "front" (+y, towards the audience) and
    // "stage right" (+x).
    let front = v.vec_to_screen(P::new(0.0, 1.0)).normalized();
    let right = v.vec_to_screen(P::new(1.0, 0.0)).normalized();
    let mut front_offset = 10.0;
    if set.is_distance_marks_shown {
        let size = (v.scale * 0.32).clamp(9.0, 13.0);
        for i in 0..=(stage.width().floor() as i64) {
            let x = stage.min.x + i as f64;
            let d = (x - mid_x).abs();
            let pos = v.to_screen(P::new(x, outer.max.y)) + front * 9.0;
            painter.text(pos, Align2::CENTER_CENTER, fmt_mark(d), FontId::monospace(size), theme::TEXT_WEAK);
        }
        front_offset += 14.0;
    }
    if !set.is_scene_directions_shown {
        return;
    }
    let font = FontId::proportional(11.5);
    let col = theme::TEXT_FAINT;
    let mid = outer.center();
    let audience = v.to_screen(P::new(mid.x, outer.max.y)) + front * front_offset;
    let back = v.to_screen(P::new(mid.x, outer.min.y)) - front * 12.0;
    painter.text(audience, Align2::CENTER_CENTER, "AUDIENCE", font.clone(), theme::TEXT_WEAK);
    painter.text(back, Align2::CENTER_CENTER, "BACKSTAGE", font.clone(), col);
    // Side labels run along the edges, reading bottom-to-top on the left side
    // of the screen and top-to-bottom on the right.
    let sr = v.to_screen(P::new(outer.max.x, mid.y)) + right * 12.0;
    let sl = v.to_screen(P::new(outer.min.x, mid.y)) - right * 12.0;
    let angle_for = |screen_dir_x: f32| if screen_dir_x > 0.0 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 };
    rotated_text(painter, sr, "STAGE RIGHT", font.clone(), col, angle_for(right.x));
    rotated_text(painter, sl, "STAGE LEFT", font, col, angle_for(-right.x));
}

/// Custom image as a textured quad mapped onto stage cells (so it turns with
/// the view).
fn paint_image(painter: &Painter, v: &View, target: Bounds, img: BgImage, aspect: StageAspectRatio, alpha: f32) {
    let (tw, th) = (target.width(), target.height());
    let ia = (img.size.x / img.size.y.max(1.0)) as f64;
    let ta = tw / th.max(1e-6);
    let mut quad = target;
    let mut uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match aspect {
        StageAspectRatio::StretchToFill => {}
        StageAspectRatio::ScaleToFit => {
            let (w, h) = if ia > ta { (tw, tw / ia) } else { (th * ia, th) };
            let c = target.center();
            quad = Bounds::new(P::new(c.x - w / 2.0, c.y - h / 2.0), P::new(c.x + w / 2.0, c.y + h / 2.0));
        }
        StageAspectRatio::ScaleToFill => {
            if ia > ta {
                let f = (ta / ia) as f32; // visible width fraction
                uv = Rect::from_min_max(pos2(0.5 - f / 2.0, 0.0), pos2(0.5 + f / 2.0, 1.0));
            } else {
                let f = (ia / ta) as f32;
                uv = Rect::from_min_max(pos2(0.0, 0.5 - f / 2.0), pos2(1.0, 0.5 + f / 2.0));
            }
        }
    }
    let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
    let mut mesh = Mesh::with_texture(img.texture);
    let corners = [
        (P::new(quad.min.x, quad.min.y), pos2(uv.min.x, uv.min.y)),
        (P::new(quad.max.x, quad.min.y), pos2(uv.max.x, uv.min.y)),
        (P::new(quad.max.x, quad.max.y), pos2(uv.max.x, uv.max.y)),
        (P::new(quad.min.x, quad.max.y), pos2(uv.min.x, uv.max.y)),
    ];
    for (p, uv) in corners {
        mesh.vertices.push(Vertex { pos: v.to_screen(p), uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// Simple procedural versions of the app's built-in backgrounds.
fn paint_builtin(painter: &Painter, v: &View, t: Bounds, kind: DefaultStageBackground, alpha: f32) {
    use DefaultStageBackground::*;
    let a = alpha.clamp(0.0, 1.0);
    // A frame where u runs along the long axis and w across, both 0..1.
    let long_x = t.width() >= t.height();
    let map = |u: f64, w: f64| -> Pos2 {
        let (fx, fy) = if long_x { (u, w) } else { (w, u) };
        v.to_screen(P::new(t.min.x + fx * t.width(), t.min.y + fy * t.height()))
    };
    let (long, short) = if long_x { (t.width(), t.height()) } else { (t.height(), t.width()) };
    let k = (short / long) as f32; // u-units per w-unit, for circles
    let white = Color32::from_white_alpha((200.0 * a) as u8);
    let line = Stroke::new((v.scale * 0.06).clamp(1.0, 3.0), white);
    let poly = |pts: &[(f64, f64)]| {
        let p: Vec<Pos2> = pts.iter().map(|&(u, w)| map(u, w)).collect();
        painter.add(Shape::closed_line(p, line));
    };
    let seg = |a: (f64, f64), b: (f64, f64)| {
        painter.line_segment([map(a.0, a.1), map(b.0, b.1)], line);
    };
    let ellipse = |cu: f64, cw: f64, r_w: f64, from: f64, to: f64, closed: bool| {
        // Circle of radius r_w (in w units) centred at (cu, cw); angles in turns.
        let n = 40;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let ang = (from + (to - from) * i as f64 / n as f64) * std::f64::consts::TAU;
                map(cu + r_w * k as f64 * ang.cos(), cw + r_w * ang.sin())
            })
            .collect();
        if closed {
            painter.add(Shape::closed_line(pts, line));
        } else {
            painter.add(Shape::line(pts, line));
        }
    };
    let floor = |col: Color32| {
        let r = v.bounds_to_screen(t);
        painter.rect_filled(r, 0.0, col.gamma_multiply(a));
    };
    match kind {
        BasketballField => {
            floor(Color32::from_rgb(0x8A, 0x5E, 0x38));
            poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
            seg((0.5, 0.0), (0.5, 1.0));
            ellipse(0.5, 0.5, 0.12, 0.0, 1.0, true);
            for end in [0.0, 1.0] {
                let dir = if end == 0.0 { 1.0 } else { -1.0 };
                let d = 0.2;
                poly(&[(end, 0.33), (end + dir * d, 0.33), (end + dir * d, 0.67), (end, 0.67)]);
                ellipse(end + dir * d, 0.5, 0.12, 0.0, 1.0, true);
                // Three-point line: corners then an arc around the basket.
                let bu = end + dir * 0.055;
                let (from, to) = if dir > 0.0 { (-0.25, 0.25) } else { (0.25, 0.75) };
                ellipse(bu, 0.5, 0.44, from, to, false);
            }
        }
        FootballField => {
            floor(Color32::from_rgb(0x2E, 0x7D, 0x32));
            let stripes = 10;
            for i in (0..stripes).step_by(2) {
                let (u0, u1) = (i as f64 / stripes as f64, (i + 1) as f64 / stripes as f64);
                let r = Rect::from_two_pos(map(u0, 0.0), map(u1, 1.0));
                painter.rect_filled(r, 0.0, Color32::from_black_alpha((22.0 * a) as u8));
            }
            poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
            seg((0.5, 0.0), (0.5, 1.0));
            ellipse(0.5, 0.5, 0.14, 0.0, 1.0, true);
            for end in [0.0, 1.0] {
                let dir = if end == 0.0 { 1.0 } else { -1.0 };
                poly(&[(end, 0.2), (end + dir * 0.16, 0.2), (end + dir * 0.16, 0.8), (end, 0.8)]);
                poly(&[(end, 0.37), (end + dir * 0.055, 0.37), (end + dir * 0.055, 0.63), (end, 0.63)]);
            }
        }
        NflField | NflFieldSmall => {
            floor(Color32::from_rgb(0x2F, 0x7A, 0x3A));
            let ez = 10.0 / 120.0;
            for (u0, u1) in [(0.0, ez), (1.0 - ez, 1.0)] {
                let r = Rect::from_two_pos(map(u0, 0.0), map(u1, 1.0));
                painter.rect_filled(r, 0.0, Color32::from_rgb(0x23, 0x5E, 0x2C).gamma_multiply(a));
            }
            poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
            let step = if kind == NflField { 5 } else { 10 };
            let mut yard = 0;
            while yard <= 100 {
                let u = ez + yard as f64 / 120.0;
                seg((u, 0.0), (u, 1.0));
                if yard % 10 == 0 && yard > 0 && yard < 100 {
                    let n = if yard <= 50 { yard } else { 100 - yard };
                    let size = (v.scale * 0.45).clamp(8.0, 26.0);
                    for w in [0.12, 0.88] {
                        painter.text(map(u, w), Align2::CENTER_CENTER, n.to_string(), FontId::proportional(size), white);
                    }
                }
                yard += step;
            }
        }
        SevenCheerleadingMats | EightCheerleadingMats | NineCheerleadingMats | TenCheerleadingMats => {
            let n = match kind {
                SevenCheerleadingMats => 7,
                EightCheerleadingMats => 8,
                NineCheerleadingMats => 9,
                _ => 10,
            };
            floor(Color32::from_rgb(0x1F, 0x4E, 0xA8));
            // Mats run front to back, side by side across the stage width.
            let mat = |i: usize| -> Rect {
                let (x0, x1) = (t.min.x + t.width() * i as f64 / n as f64, t.min.x + t.width() * (i + 1) as f64 / n as f64);
                v.bounds_to_screen(Bounds::new(P::new(x0, t.min.y), P::new(x1, t.max.y)))
            };
            for i in 0..n {
                let r = mat(i);
                if i % 2 == 1 {
                    painter.rect_filled(r, 0.0, Color32::from_black_alpha((26.0 * a) as u8));
                }
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha((70.0 * a) as u8)), StrokeKind::Inside);
                let front = v.to_screen(P::new(t.min.x + t.width() * (i as f64 + 0.5) / n as f64, t.min.y + 0.45));
                let size = (v.scale * 0.36).clamp(8.0, 20.0);
                painter.text(front, Align2::CENTER_CENTER, (i + 1).to_string(), FontId::proportional(size), white);
            }
        }
        Unknown => {
            floor(Color32::from_rgb(0x34, 0x2C, 0x48));
            let size = (v.scale * 0.4).clamp(9.0, 16.0);
            painter.text(
                v.to_screen(P::new(t.center().x, t.max.y - 0.5)),
                Align2::CENTER_CENTER,
                "Stage background",
                FontId::proportional(size),
                Color32::from_white_alpha((90.0 * a) as u8),
            );
        }
    }
}

/// Human name of a built-in background.
pub fn builtin_label(b: DefaultStageBackground) -> &'static str {
    use DefaultStageBackground::*;
    match b {
        Unknown => "Plain floor",
        BasketballField => "Basketball court",
        FootballField => "Football pitch",
        NflField => "American football field",
        NflFieldSmall => "American football field (small)",
        SevenCheerleadingMats => "7 cheer mats",
        EightCheerleadingMats => "8 cheer mats",
        NineCheerleadingMats => "9 cheer mats",
        TenCheerleadingMats => "10 cheer mats",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_sit_on_the_prop() {
        let c = Choreography::new("t", "id", 0);
        let view = View::fit(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 800.0)), geom::stage_bounds(&c), 0.0, false);
        let mut p = crate::edit::new_prop(&c, arrus_core::PropType::Rectangle);
        p.position = P::new(7.0, 4.0);
        p.width = 2.0;
        p.length = 1.0;
        let (corners, knob, front) = prop_handles(&p, &view);
        assert_eq!(corners[0], pos2(600.0, 350.0));
        assert_eq!(corners[2], pos2(800.0, 450.0));
        assert_eq!(front, pos2(700.0, 350.0));
        assert!((knob - pos2(700.0, 350.0 - ROTATION_HANDLE_OFFSET)).length() < 1e-3);
    }

    #[test]
    fn fit_is_the_grid_plus_stray_dancers() {
        let mut c = Choreography::new("t", "id", 0);
        assert_eq!(fit_bounds(&c), geom::drawable_bounds(&c));
        // The backstage is inside the grid: the fit does not grow.
        c.grid_size.backstage.back = 2;
        assert_eq!(fit_bounds(&c), geom::drawable_bounds(&c));
        // A dancer outside the grid (older web documents) stays in view.
        c.add_dancer(arrus_core::Unit::at(-1.5, 3.0));
        assert_eq!(fit_bounds(&c).min, P::new(-2.0, 0.0));
        assert_eq!(dancer_radius(&c), 0.35);
    }
}
