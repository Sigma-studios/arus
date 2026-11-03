//! Bottom bar: transport controls, time, music, and the formation timeline.

use arrus_core::timeline::{self, Phase};
use egui::{Align, Align2, Color32, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::editor::Editor;
use crate::view::{time_to_x, x_to_time};
use crate::{edit, theme, util};

pub enum MusicAction {
    Upload,
    Remove,
}

const NO_MUSIC_EXPORT: &str = "The ArrangeUs phone app doesn't export music, so upload the track here. \
                               Playback then follows the music.";

pub fn show(ed: &mut Editor, ui: &mut Ui) -> Option<MusicAction> {
    if ed.compact {
        show_compact(ed, ui);
        return None;
    }
    let mut action = None;
    let compact = ed.compact;
    egui::Panel::bottom("timeline")
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(14, 10)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let size = if compact { vec2(46.0, 40.0) } else { vec2(36.0, 30.0) };
                if transport_button(ui, size, Icon::Prev, None).on_hover_text("Previous formation (←)").clicked() {
                    ed.prev_formation();
                }
                let icon = if ed.playing { Icon::Pause } else { Icon::Play };
                if transport_button(ui, size + vec2(8.0, 0.0), icon, Some(theme::ACCENT))
                    .on_hover_text("Play / pause (Space)")
                    .clicked()
                {
                    ed.toggle_play();
                }
                if transport_button(ui, size, Icon::Next, None).on_hover_text("Next formation (→)").clicked() {
                    ed.next_formation();
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("{} / {}", util::fmt_time(ed.t), util::fmt_time(ed.timeline_len())))
                        .monospace()
                        .size(if compact { 15.0 } else { 14.0 }),
                );
                ui.add_space(8.0);
                let label = formation_label(ed);
                if !compact {
                    ui.label(RichText::new(&label).color(theme::TEXT_WEAK));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if compact {
                        if let Some(m) = &ed.summary.music {
                            ui.add(egui::Label::new(RichText::new(format!("♪ {}", m.filename)).small().color(theme::TEXT_WEAK)).truncate());
                        }
                        return;
                    }
                    if ed.music_busy {
                        ui.spinner();
                    }
                    match (&ed.summary.music, ed.can_edit) {
                        (Some(m), true) => {
                            if ui.add_enabled(!ed.music_busy, egui::Button::new("Remove")).clicked() {
                                action = Some(MusicAction::Remove);
                            }
                            if ui.add_enabled(!ed.music_busy, egui::Button::new("Replace…")).on_hover_text(NO_MUSIC_EXPORT).clicked() {
                                action = Some(MusicAction::Upload);
                            }
                            music_label(ui, &m.filename, m.bytes);
                        }
                        (Some(m), false) => music_label(ui, &m.filename, m.bytes),
                        (None, true) => {
                            if ui.add_enabled(!ed.music_busy, egui::Button::new("♪ Add music…")).clicked() {
                                action = Some(MusicAction::Upload);
                            }
                            ui.label(RichText::new("ⓘ").color(theme::TEXT_FAINT)).on_hover_text(NO_MUSIC_EXPORT);
                            ui.label(RichText::new("The phone app doesn't export music").small().color(theme::TEXT_FAINT));
                        }
                        (None, false) => {
                            ui.label(RichText::new("No music").small().color(theme::TEXT_FAINT));
                        }
                    }
                });
            });
            if compact {
                let label = formation_label(ed);
                ui.add(egui::Label::new(RichText::new(label).color(theme::TEXT_WEAK)).truncate());
            }
            ui.add_space(6.0);
            bar(ed, ui);
        });
    action
}

/// Phone layout: one slim row of transport, time and formation, then a thin
/// timeline. Music is managed on a computer. While a sheet (formations or
/// details) is open, only previous / formation / next are kept.
fn show_compact(ed: &mut Editor, ui: &mut Ui) {
    if ed.show_formations || ed.show_inspector {
        egui::Panel::bottom("timeline")
            .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(10, 5)))
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.y = 30.0;
                ui.spacing_mut().button_padding = vec2(6.0, 4.0);
                ui.horizontal(|ui| {
                    let size = vec2(44.0, 30.0);
                    if transport_button(ui, size, Icon::Prev, None).clicked() {
                        ed.prev_formation();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if transport_button(ui, size, Icon::Next, None).clicked() {
                            ed.next_formation();
                        }
                        // Playback started before opening the sheet can still be stopped.
                        if ed.playing && transport_button(ui, size, Icon::Pause, Some(theme::ACCENT)).clicked() {
                            ed.toggle_play();
                        }
                        ui.centered_and_justified(|ui| {
                            ui.add(egui::Label::new(RichText::new(formation_label(ed)).size(13.5)).truncate());
                        });
                    });
                });
            });
        return;
    }
    egui::Panel::bottom("timeline")
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(10, 6)))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 30.0;
            ui.spacing_mut().button_padding = vec2(6.0, 4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let size = vec2(38.0, 30.0);
                if transport_button(ui, size, Icon::Prev, None).clicked() {
                    ed.prev_formation();
                }
                let icon = if ed.playing { Icon::Pause } else { Icon::Play };
                if transport_button(ui, size, icon, Some(theme::ACCENT)).clicked() {
                    ed.toggle_play();
                }
                if transport_button(ui, size, Icon::Next, None).clicked() {
                    ed.next_formation();
                }
                ui.label(
                    RichText::new(format!("{} / {}", util::fmt_time(ed.t), util::fmt_time(ed.timeline_len())))
                        .monospace()
                        .size(12.5)
                        .color(theme::TEXT_WEAK),
                );
                ui.add(egui::Label::new(RichText::new(formation_label(ed)).size(13.5)).truncate());
            });
            ui.add_space(8.0);
            bar(ed, ui);
        });
}

fn formation_label(ed: &Editor) -> String {
    match timeline::phase_at(&ed.doc, ed.t, ed.easing) {
        Some(Phase::Transition { from, to, .. }) => format!("Transition {} → {}", from + 1, to + 1),
        Some(Phase::Hold { formation }) => format!("Formation {}", edit::formation_label(&ed.doc, formation)),
        None => String::new(),
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Prev,
    Next,
    Play,
    Pause,
}

/// A button with a painted transport icon (font-independent).
fn transport_button(ui: &mut Ui, size: egui::Vec2, icon: Icon, fill: Option<Color32>) -> egui::Response {
    let mut b = egui::Button::new("").min_size(size);
    if let Some(f) = fill {
        b = b.fill(f);
    }
    let r = ui.add(b);
    let c = r.rect.center();
    let s = r.rect.height().min(r.rect.width()) * 0.2;
    let col = if fill.is_some() || r.hovered() { Color32::WHITE } else { theme::TEXT };
    let p = ui.painter();
    let tri = |pts: Vec<egui::Pos2>| egui::Shape::convex_polygon(pts, col, Stroke::NONE);
    match icon {
        Icon::Play => {
            p.add(tri(vec![pos2(c.x - s * 0.75, c.y - s), pos2(c.x + s, c.y), pos2(c.x - s * 0.75, c.y + s)]));
        }
        Icon::Pause => {
            for dx in [-0.55, 0.55] {
                let r = Rect::from_center_size(pos2(c.x + dx * s, c.y), vec2(s * 0.5, s * 2.0));
                p.rect_filled(r, 1.0, col);
            }
        }
        Icon::Prev | Icon::Next => {
            let d = if matches!(icon, Icon::Prev) { -1.0 } else { 1.0 };
            p.add(tri(vec![pos2(c.x - d * s * 0.7, c.y - s), pos2(c.x + d * s * 0.7, c.y), pos2(c.x - d * s * 0.7, c.y + s)]));
            let bar = Rect::from_center_size(pos2(c.x + d * s * 0.95, c.y), vec2(s * 0.35, s * 2.0));
            p.rect_filled(bar, 1.0, col);
        }
    }
    r
}

fn music_label(ui: &mut Ui, name: &str, bytes: u64) {
    ui.label(RichText::new(format!("♪ {name}")).color(theme::TEXT)).on_hover_text(format!(
        "{} · {}",
        util::fmt_bytes(bytes),
        NO_MUSIC_EXPORT
    ));
}

fn bar(ed: &mut Editor, ui: &mut Ui) {
    let h = if ed.compact { 30.0 } else { 42.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
    let len = ed.timeline_len();
    let p = ui.painter_at(rect.expand(8.0));
    p.rect_filled(rect, 6.0, theme::BG);
    let spans = timeline::spans(&ed.doc);
    let cur = ed.cur();
    let inner = rect.shrink2(vec2(0.0, 4.0));
    for s in &spans {
        let x0 = time_to_x(s.start, len, rect);
        let x1 = time_to_x(s.hold_start, len, rect);
        let x2 = time_to_x(s.end, len, rect);
        if x1 > x0 + 0.5 {
            let tr = Rect::from_min_max(pos2(x0, inner.top()), pos2(x1, inner.bottom()));
            p.rect_filled(tr, 3.0, theme::TRANSITION);
            // Diagonal hatching marks a transition.
            let clip = p.with_clip_rect(tr);
            let mut x = tr.left() - tr.height();
            while x < tr.right() {
                clip.line_segment(
                    [pos2(x, tr.bottom()), pos2(x + tr.height(), tr.top())],
                    Stroke::new(1.0, Color32::from_white_alpha(14)),
                );
                x += 7.0;
            }
        }
        if x2 > x1 + 0.5 {
            let hr = Rect::from_min_max(pos2(x1 + 1.0, inner.top()), pos2(x2 - 1.0, inner.bottom()));
            let selected = s.formation == cur;
            p.rect_filled(hr, 4.0, if selected { theme::HOLD_CURRENT } else { theme::HOLD });
            if selected {
                p.rect_stroke(hr, 4.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
            let f = &ed.doc.formations[s.formation];
            let stripe = Rect::from_min_max(pos2(hr.left() + 2.0, hr.bottom() - 3.0), pos2(hr.right() - 2.0, hr.bottom() - 1.0));
            if let Some(c) = f.color {
                p.rect_filled(stripe, 1.0, theme::argb(c, 0.8));
            }
            if hr.width() > 18.0 {
                let mut text = (s.formation + 1).to_string();
                if let Some(n) = f.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
                    && hr.width() > 70.0
                {
                    text = format!("{text} · {n}");
                }
                p.with_clip_rect(hr.shrink(2.0)).text(
                    hr.left_center() + vec2(6.0, -1.0),
                    Align2::LEFT_CENTER,
                    text,
                    FontId::proportional(12.5),
                    if selected { theme::TEXT } else { theme::TEXT_WEAK },
                );
            }
        }
    }
    // Music beyond the last formation.
    let total = ed.total();
    if len > total + 0.01 {
        let r = Rect::from_min_max(pos2(time_to_x(total, len, rect) + 1.0, inner.top()), pos2(rect.right(), inner.bottom()));
        p.rect_filled(r, 3.0, Color32::from_white_alpha(6));
        if r.width() > 60.0 {
            p.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, "music continues", FontId::proportional(11.5), theme::TEXT_FAINT);
        }
    }

    // Seeking.
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.is_pointer_button_down_on() || resp.clicked())
    {
        ed.seek(x_to_time(pos.x, len, rect));
    }
    ed.scrubbing = resp.is_pointer_button_down_on();
    if let Some(hp) = resp.hover_pos() {
        let ht = x_to_time(hp.x, len, rect);
        p.line_segment([pos2(hp.x, rect.top()), pos2(hp.x, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(40)));
        resp.clone().on_hover_text_at_pointer(util::fmt_time(ht));
    }

    // Playhead.
    let x = time_to_x(ed.t, len, rect);
    p.line_segment([pos2(x, rect.top() - 3.0), pos2(x, rect.bottom() + 3.0)], Stroke::new(2.0, theme::ACCENT));
    p.add(egui::Shape::convex_polygon(
        vec![pos2(x - 6.0, rect.top() - 6.0), pos2(x + 6.0, rect.top() - 6.0), pos2(x, rect.top() + 2.0)],
        theme::ACCENT,
        Stroke::NONE,
    ));
}
