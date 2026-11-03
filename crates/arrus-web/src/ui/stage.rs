//! The stage widget: drawing plus zoom/pan, selection and drag editing.

use std::collections::BTreeSet;

use arrus_core::PropType;
use arrus_core::timeline;
use egui::{Align, Color32, CursorIcon, Layout, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, vec2};

use super::editor::Editor;
use crate::edit::{self, Selection};
use crate::geom::{self, P, PropFrame};
use crate::paint::{self, BgImage, Scene};
use crate::view::View;
use crate::theme;

/// An in-progress pointer gesture on the stage.
#[derive(Debug, Clone)]
pub enum Drag {
    Pan,
    Box { origin: Pos2, base: BTreeSet<usize> },
    Dancers { anchor_start: P, pointer_start: P, starts: Vec<(usize, P)>, moved: bool },
    Prop { idx: usize, start: P, pointer_start: P, moved: bool },
    Resize { idx: usize, signs: (f64, f64), fixed: P, moved: bool },
    Rotate { idx: usize, moved: bool },
}

/// Margin around the fitted stage (room for edge labels).
const FIT_MARGIN: f32 = 46.0;

enum Hit {
    Knob(usize),
    Corner(usize, usize),
    Dancer(usize),
    Prop(usize),
    None,
}

pub fn show(ed: &mut Editor, ui: &mut Ui) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    // The part of the stage area actually on screen: fit and centre on that,
    // so an overflowing row elsewhere can never push the stage off-screen.
    let visible = rect.intersect(ui.ctx().content_rect());
    let visible = if visible.is_positive() { visible } else { rect };
    // As in the ArrangeUs app: by default the audience is at the top;
    // `isSceneDirectionsInverted` puts the backstage at the top.
    let inverted = !ed.doc.settings.is_scene_directions_inverted;

    // View: fit on open / on request / on resize when untouched.
    let resized = (visible.size() - ed.last_stage_size).length() > 1.0;
    ed.last_stage_size = visible.size();
    // Room for the floating toolbar at the top.
    let top = if ed.compact { 0.0 } else { 40.0 };
    let fit_rect = Rect::from_min_max(visible.min + vec2(0.0, top), visible.max);
    let margin = if ed.compact { 34.0 } else { FIT_MARGIN };
    if ed.view.is_none() || ed.need_fit || (resized && !ed.view_moved) {
        ed.view = Some(View::fit(fit_rect, paint::fit_bounds(&ed.doc), margin, inverted));
        ed.need_fit = false;
        ed.view_moved = false;
    }
    let mut view = ed.view.unwrap_or_else(|| View::fit(fit_rect, paint::fit_bounds(&ed.doc), margin, inverted));
    view.set_screen_center(fit_rect.center());
    view.set_inverted(inverted);

    // Zoom (wheel, ctrl-wheel, pinch) and two-finger pan.
    let (zoom, scroll, touch, hover, shift, middle, secondary) = ui.input(|i| {
        (
            i.zoom_delta(),
            i.smooth_scroll_delta(),
            i.multi_touch(),
            i.pointer.hover_pos(),
            i.modifiers.shift,
            i.pointer.button_down(PointerButton::Middle),
            i.pointer.button_down(PointerButton::Secondary),
        )
    });
    // Touch has no Shift key: the multi-select toggle stands in for it.
    let shift = shift || ed.multi_select;
    if resp.contains_pointer() || touch.is_some() {
        if let Some(t) = &touch {
            view.zoom_at(t.center_pos, t.zoom_delta);
            view.pan(t.translation_delta);
            ed.view_moved = true;
        } else {
            let anchor = hover.unwrap_or(rect.center());
            if zoom != 1.0 {
                view.zoom_at(anchor, zoom);
                ed.view_moved = true;
            }
            if scroll.y != 0.0 {
                view.zoom_at(anchor, (scroll.y * 0.0025).exp());
                ed.view_moved = true;
            }
        }
    }

    let f = ed.cur();
    let editing = ed.can_edit && !ed.playing;
    let dancers = timeline::dancers_at(&ed.doc, ed.t, ed.easing);
    let shown_props = edit::props_at(&ed.doc, ed.t, ed.easing);
    let positions: Vec<(usize, P)> = dancers.iter().filter(|d| d.opacity > 0.5).map(|d| (d.index, d.position)).collect();
    let radius = paint::dancer_radius(&ed.doc);
    let min_px = if ed.compact { 18.0 } else { 9.0 };
    let hit_r = radius.max(min_px / view.scale as f64);
    let tol = 4.0 / view.scale as f64;
    let snap = ed.doc.settings.is_grid_snapping_enabled.then_some(geom::SNAP_STEP);
    let bounds = geom::drawable_bounds(&ed.doc);

    let hit_at = |ed: &Editor, view: &View, sp: Pos2| -> Hit {
        let w = view.stage_at(sp);
        if editing
            && let Selection::Prop(i) = ed.sel
            && let Some(p) = edit::props(&ed.doc, f).get(i)
        {
            let (corners, knob, _) = paint::prop_handles(p, view);
            if knob.distance(sp) <= paint::HANDLE_HALF + 6.0 {
                return Hit::Knob(i);
            }
            if let Some(k) = corners.iter().position(|c| c.distance(sp) <= paint::HANDLE_HALF + 5.0) {
                return Hit::Corner(i, k);
            }
        }
        if let Some(i) = edit::hit_dancer(&positions, w, hit_r) {
            return Hit::Dancer(i);
        }
        if let Some(i) = edit::hit_prop(&shown_props, w, tol) {
            return Hit::Prop(i);
        }
        Hit::None
    };

    // Hover feedback.
    let mut hover_dancer = None;
    let mut hover_prop = None;
    if ed.drag.is_none()
        && let Some(hp) = resp.hover_pos()
    {
        match hit_at(ed, &view, hp) {
            Hit::Dancer(i) => {
                hover_dancer = Some(i);
                if editing {
                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                }
            }
            Hit::Prop(i) => {
                hover_prop = Some(i);
                if editing {
                    ui.ctx().set_cursor_icon(CursorIcon::Move);
                }
            }
            Hit::Knob(_) => ui.ctx().set_cursor_icon(CursorIcon::Alias),
            Hit::Corner(..) => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
            Hit::None => {
                if ed.space_down {
                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                }
            }
        }
    }

    // Start of a drag.
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()).unwrap_or(rect.center());
        let touching = ui.input(|i| i.any_touches());
        let pan = middle || secondary || ed.space_down || !editing || touch.is_some();
        if ed.space_down {
            ed.space_pan_used = true;
        }
        ed.drag = Some(if pan {
            Drag::Pan
        } else {
            let w = view.stage_at(origin);
            match hit_at(ed, &view, origin) {
                Hit::Knob(idx) => Drag::Rotate { idx, moved: false },
                Hit::Corner(idx, k) => {
                    let p = &edit::props(&ed.doc, f)[idx];
                    let fixed = PropFrame::of(p).corners()[(k + 2) % 4];
                    Drag::Resize { idx, signs: geom::CORNER_SIGNS[k], fixed, moved: false }
                }
                Hit::Dancer(i) => {
                    if !ed.sel.has_dancer(i) {
                        if shift {
                            ed.sel.toggle(i);
                        } else {
                            ed.sel = Selection::one(i);
                        }
                    }
                    let sel = ed.sel.dancers().cloned().unwrap_or_default();
                    let fp = edit::formation_positions(&ed.doc, f);
                    let starts: Vec<(usize, P)> = fp.iter().filter(|(j, _)| sel.contains(j)).copied().collect();
                    let anchor_start = fp.get(i).map(|x| x.1).unwrap_or(w);
                    Drag::Dancers { anchor_start, pointer_start: w, starts, moved: false }
                }
                Hit::Prop(idx) => {
                    ed.sel = Selection::Prop(idx);
                    let start = edit::props(&ed.doc, f).get(idx).map(|p| p.position).unwrap_or(w);
                    Drag::Prop { idx, start, pointer_start: w, moved: false }
                }
                // On a touch screen one finger on empty space pans; selecting
                // with a box needs the multi-select toggle.
                Hit::None if touching && !ed.multi_select => Drag::Pan,
                Hit::None => {
                    let base = if shift { ed.sel.dancers().cloned().unwrap_or_default() } else { BTreeSet::new() };
                    if !shift {
                        ed.sel = Selection::None;
                    }
                    Drag::Box { origin, base }
                }
            }
        });
    }

    // Drag in progress.
    if let Some(mut drag) = ed.drag.take() {
        let pointer = resp.interact_pointer_pos().or(hover);
        let mut keep = resp.dragged() || resp.drag_started();
        // A second finger turns the gesture into pinch zoom: stop editing.
        let pinching = touch.is_some() && !matches!(drag, Drag::Pan);
        if let (true, Some(pp), false) = (resp.dragged(), pointer, pinching) {
            let w = view.stage_at(pp);
            match &mut drag {
                Drag::Pan => {
                    if touch.is_none() {
                        view.pan(resp.drag_delta());
                        ed.view_moved = true;
                    }
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                Drag::Box { origin, base } => {
                    let mut s = base.clone();
                    s.extend(edit::box_select(&positions, view.stage_at(*origin), w));
                    ed.sel = if s.is_empty() { Selection::None } else { Selection::Dancers(s) };
                }
                Drag::Dancers { anchor_start, pointer_start, starts, moved } => {
                    if !*moved {
                        ed.begin_gesture();
                        *moved = true;
                    }
                    let delta = geom::sub(w, *pointer_start);
                    edit::drag_dancers(&mut ed.doc, f, starts, *anchor_start, delta, snap, bounds);
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                Drag::Prop { idx, start, pointer_start, moved } => {
                    if !*moved {
                        ed.begin_gesture();
                        *moved = true;
                    }
                    let mut to = geom::add(*start, geom::sub(w, *pointer_start));
                    if let Some(s) = snap {
                        to = geom::snap_pos(to, s);
                    }
                    if let Some(p) = edit::prop_mut(&mut ed.doc, f, *idx) {
                        p.position = bounds.clamp(to);
                    }
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                Drag::Resize { idx, signs, fixed, moved } => {
                    if !*moved {
                        ed.begin_gesture();
                        *moved = true;
                    }
                    if let Some(p) = edit::prop_mut(&mut ed.doc, f, *idx) {
                        let r = geom::resize_from_corner(*signs, *fixed, w, p.rotation_angle, snap);
                        p.position = r.center;
                        p.width = r.width;
                        p.length = r.length;
                    }
                }
                Drag::Rotate { idx, moved } => {
                    if !*moved {
                        ed.begin_gesture();
                        *moved = true;
                    }
                    if let Some(p) = edit::prop_mut(&mut ed.doc, f, *idx) {
                        let step = (snap.is_some() && !shift).then_some(geom::ROTATION_SNAP_DEG);
                        p.rotation_angle = geom::rotation_towards(p.position, w, step);
                    }
                    ui.ctx().set_cursor_icon(CursorIcon::Alias);
                }
            }
        }
        if resp.drag_stopped() {
            keep = false;
            ed.end_gesture();
        }
        if keep {
            ed.drag = Some(drag);
        } else if !matches!(drag, Drag::Pan | Drag::Box { .. }) {
            ed.end_gesture();
        }
    }

    // Clicks.
    if resp.clicked()
        && let Some(pp) = resp.interact_pointer_pos()
    {
        match hit_at(ed, &view, pp) {
            Hit::Dancer(i) => {
                if shift {
                    ed.sel.toggle(i);
                } else {
                    ed.sel = Selection::one(i);
                }
            }
            Hit::Prop(i) => ed.sel = Selection::Prop(i),
            Hit::Knob(_) | Hit::Corner(..) => {}
            Hit::None => {
                if !shift {
                    ed.sel = Selection::None;
                }
            }
        }
    }
    if resp.double_clicked() && matches!(resp.interact_pointer_pos().map(|p| hit_at(ed, &view, p)), Some(Hit::None)) {
        ed.need_fit = true;
    }

    ed.view = Some(view);

    // Paint.
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::BG);
    let background = ed.bg_tex.as_ref().map(|(_, t)| BgImage { texture: t.id(), size: t.size_vec2() });
    // Redo the display after edits this frame so drags feel immediate.
    let dancers = timeline::dancers_at(&ed.doc, ed.t, ed.easing);
    let shown_props = edit::props_at(&ed.doc, ed.t, ed.easing);
    let scene = Scene {
        c: &ed.doc,
        view: &view,
        formation: ed.cur(),
        dancers: &dancers,
        props: &shown_props,
        selection: &ed.sel,
        hover_dancer,
        hover_prop,
        background,
        handles: editing,
        playing: ed.playing,
    };
    paint::paint_stage(&painter, &scene);

    match &ed.drag {
        Some(Drag::Box { origin, .. }) => {
            if let Some(pp) = resp.interact_pointer_pos() {
                let r = Rect::from_two_pos(*origin, pp);
                painter.rect_filled(r, 2.0, theme::SELECT.gamma_multiply(0.12));
                painter.rect_stroke(r, 2.0, Stroke::new(1.0, theme::SELECT), egui::StrokeKind::Inside);
            }
        }
        Some(Drag::Dancers { starts, .. }) if ed.doc.settings.is_additional_indicators_shown => {
            if let (Some(pp), Some((i, _))) = (resp.interact_pointer_pos(), starts.first())
                && let Some(u) = ed.doc.formations.get(f).and_then(|fm| fm.units.get(*i))
            {
                let text = format!("x {:.1} · y {:.1}", u.position.x, u.position.y);
                let at = pp + vec2(16.0, 16.0);
                let g = painter.layout_no_wrap(text, egui::FontId::monospace(12.0), theme::TEXT);
                let r = Rect::from_min_size(at, g.size()).expand(5.0);
                painter.rect_filled(r, 4.0, Color32::from_black_alpha(200));
                painter.galley(at, g, theme::TEXT);
            }
        }
        _ => {}
    }

    overlay(ed, ui, visible);
}

/// Floating tools in the stage's top-left corner. Phones get them in the
/// editor's tool row instead (nothing floats over the stage there).
fn overlay(ed: &mut Editor, ui: &mut Ui, rect: Rect) {
    if ed.compact {
        return;
    }
    let bar = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(rect.width() - 20.0, 36.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
    let ui = &mut child;
    let frame = egui::Frame::new()
        .fill(theme::SURFACE.gamma_multiply(0.92))
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(6, 4));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Fit").on_hover_text("Fit the stage in view (or double-click empty space)").clicked() {
                ed.need_fit = true;
            }
            if let Some(v) = &mut ed.view {
                if ui.small_button("−").clicked() {
                    v.zoom_at(rect.center(), 1.0 / 1.25);
                    ed.view_moved = true;
                }
                if !ed.compact {
                    ui.label(RichText::new(format!("{:.0} px/cell", v.scale)).small().color(theme::TEXT_WEAK));
                }
                if ui.small_button("+").clicked() {
                    v.zoom_at(rect.center(), 1.25);
                    ed.view_moved = true;
                }
            }
            ui.separator();
            let mut inv = ed.doc.settings.is_scene_directions_inverted;
            let label = if ed.compact { "Audience view" } else { "View from audience" };
            if ui.toggle_value(&mut inv, label).on_hover_text("Rotate the stage 180°: backstage at the top").changed() {
                let mut s = ed.doc.settings.clone();
                s.is_scene_directions_inverted = inv;
                ed.set_settings(s, None);
            }
        });
    });
    if ed.can_edit {
        ui.add_space(8.0);
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("+ Dancer").on_hover_text("Add a dancer to every formation").clicked() {
                    ed.pause();
                    ed.add_dancer();
                }
                ui.menu_button("+ Prop", |ui| {
                    for &k in PropType::ALL {
                        if ui.button(geom::prop_label(k)).clicked() {
                            ed.pause();
                            ed.add_prop(k);
                            ui.close();
                        }
                    }
                });
            });
        });
        if let Some(s) = ed.sel.dancers() {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} selected", s.len())).small().color(theme::TEXT_WEAK));
        }
    }
}
