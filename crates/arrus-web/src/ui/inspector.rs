//! Right panel: selected dancer(s), selected prop, or choreography settings.

use std::collections::BTreeSet;

use arrus_core::timeline::Easing;
use arrus_core::{
    Argb, DancerFillStyle, DancerShape, DefaultStageBackground, PropType, Settings, StageAspectRatio, StageBackground,
    StageBackgroundFile,
};
use egui::color_picker::Alpha;
use egui::{Color32, Frame, Margin, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

use super::App;
use super::editor::{self, Editor};
use super::widgets;
use crate::edit::{self, Selection};
use crate::geom;
use crate::paint;
use crate::theme;

pub fn show(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    egui::Panel::right("inspector")
        .resizable(true)
        .default_size(300.0)
        .size_range(260.0..=440.0)
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::same(12)))
        .show(ui, |ui| body(ed, app, ui));
}

/// Phone layout: the same content as a resizable sheet at the bottom.
pub fn show_sheet(ed: &mut Editor, app: &mut App, ui: &mut Ui, open: &mut bool) {
    let h = ui.ctx().content_rect().height();
    egui::Panel::bottom("inspector_sheet")
        .resizable(true)
        .default_size(h * 0.4)
        .size_range(140.0..=(h * 0.75).max(160.0))
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::same(12)))
        .show_collapsible(ui, open, |ui| body(ed, app, ui));
}

fn body(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        match ed.sel.clone() {
            Selection::Dancers(s) if s.len() == 1 => dancer(ed, ui, *s.first().unwrap_or(&0)),
            Selection::Dancers(s) if !s.is_empty() => dancers(ed, ui, &s),
            Selection::Prop(i) => prop(ed, ui, i),
            _ => settings(ed, app, ui),
        }
    });
}

fn to_c32(v: i64) -> Color32 {
    let a = Argb::from_i64(v);
    Color32::from_rgb(a.r, a.g, a.b)
}

fn from_c32(c: Color32) -> i64 {
    Argb { a: 255, r: c.r(), g: c.g(), b: c.b() }.to_i64()
}

fn header(ui: &mut Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).strong().size(16.0));
    if !sub.is_empty() {
        ui.label(RichText::new(sub).small().color(theme::TEXT_WEAK));
    }
    ui.add_space(4.0);
}

/// Shape buttons; returns the picked shape.
fn shape_picker(ui: &mut Ui, current: Option<DancerShape>, color: Color32, enabled: bool) -> Option<DancerShape> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for &s in DancerShape::ALL {
            let sense = if enabled { Sense::click() } else { Sense::hover() };
            let (r, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), sense);
            let selected = current == Some(s);
            let fill = if selected {
                theme::ACCENT_DIM
            } else if resp.hovered() && enabled {
                theme::HOVER
            } else {
                theme::RAISED
            };
            let p = ui.painter();
            p.rect_filled(r, 6.0, fill);
            if selected {
                p.rect_stroke(r, 6.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
            paint::shape_icon(p, r, s, color);
            if resp.on_hover_text(geom::shape_label(s)).clicked() {
                picked = Some(s);
            }
        }
    });
    picked
}

fn dancer(ed: &mut Editor, ui: &mut Ui, i: usize) {
    let f = ed.cur();
    let Some(u) = ed.doc.formations.get(f).and_then(|fm| fm.units.get(i)).cloned() else { return };
    let can = ed.can_edit;
    header(ui, &format!("Dancer {}", i + 1), "Name, label, colour and shape apply in every formation.");

    widgets::section(ui, "Identity");
    egui::Grid::new("dancer_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label("Name");
        let mut name = u.dancer.name.clone();
        if ui
            .add_enabled(can, egui::TextEdit::singleline(&mut name).hint_text("Name").desired_width(170.0))
            .changed()
        {
            ed.edit(Some(&format!("dname{i}")), |d| edit::edit_dancer_everywhere(d, i, |u| u.dancer.name = name.clone()));
        }
        ui.end_row();
        ui.label("Label");
        let mut short = u.short_name.clone().unwrap_or_default();
        if ui
            .add_enabled(can, egui::TextEdit::singleline(&mut short).hint_text("e.g. AB").char_limit(4).desired_width(70.0))
            .on_hover_text("Shown inside the dancer when the label style is “Short names”")
            .changed()
        {
            ed.edit(Some(&format!("dshort{i}")), |d| {
                edit::edit_dancer_everywhere(d, i, |u| u.short_name = (!short.is_empty()).then(|| short.clone()))
            });
        }
        ui.end_row();
        ui.label("Colour");
        let mut c = to_c32(u.color);
        ui.add_enabled_ui(can, |ui| {
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, Alpha::Opaque).changed() {
                let v = from_c32(c);
                ed.edit(Some(&format!("dcolor{i}")), |d| edit::edit_dancer_everywhere(d, i, |u| u.color = v));
            }
        });
        ui.end_row();
    });
    widgets::section(ui, "Shape");
    if let Some(s) = shape_picker(ui, Some(u.shape()), to_c32(u.color), can) {
        ed.edit(None, |d| edit::edit_dancer_everywhere(d, i, |u| u.shape = Some(s.as_str().to_owned())));
    }

    widgets::section(ui, &format!("Position in formation {}", f + 1));
    let b = geom::drawable_bounds(&ed.doc);
    ui.horizontal(|ui| {
        let mut p = u.position;
        ui.label("x");
        let rx = ui.add_enabled(can, egui::DragValue::new(&mut p.x).speed(0.05).range(b.min.x..=b.max.x).fixed_decimals(1));
        ui.label("y");
        let ry = ui.add_enabled(can, egui::DragValue::new(&mut p.y).speed(0.05).range(b.min.y..=b.max.y).fixed_decimals(1));
        if rx.changed() || ry.changed() {
            ed.edit(Some(&format!("dpos{i}")), |d| d.formations[f].units[i].position = p);
        }
    });
    ui.label(
        RichText::new("x runs across the stage (towards stage right), y from the back edge (0) to the audience.")
            .small()
            .color(theme::TEXT_FAINT),
    );
    if can {
        ui.add_space(10.0);
        if widgets::danger_button(ui, "Remove dancer…").clicked() {
            ed.modal = Some(editor::EdModal::RemoveDancers(BTreeSet::from([i])));
        }
    }
}

fn dancers(ed: &mut Editor, ui: &mut Ui, set: &BTreeSet<usize>) {
    let f = ed.cur();
    let can = ed.can_edit;
    header(ui, &format!("{} dancers", set.len()), "Changes apply to all of them, in every formation.");
    let units: Vec<_> =
        set.iter().filter_map(|&i| ed.doc.formations.get(f).and_then(|fm| fm.units.get(i)).map(|u| (i, u.clone()))).collect();
    for (i, u) in &units {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            paint::shape_icon(ui.painter(), r, u.shape(), to_c32(u.color));
            let name = if u.dancer.name.trim().is_empty() { "—" } else { u.dancer.name.as_str() };
            ui.label(format!("{}  {name}", i + 1));
        });
    }
    if !can {
        return;
    }
    widgets::section(ui, "Colour");
    let first = units.first().map(|(_, u)| u.color).unwrap_or(arrus_core::DEFAULT_COLOR);
    let mut c = to_c32(first);
    if egui::color_picker::color_edit_button_srgba(ui, &mut c, Alpha::Opaque).changed() {
        let v = from_c32(c);
        let set = set.clone();
        ed.edit(Some("mcolor"), |d| {
            for &i in &set {
                edit::edit_dancer_everywhere(d, i, |u| u.color = v);
            }
        });
    }
    widgets::section(ui, "Shape");
    let shapes: BTreeSet<&str> = units.iter().map(|(_, u)| u.shape().as_str()).collect();
    let current = (shapes.len() == 1).then(|| DancerShape::parse(shapes.first().copied().unwrap_or("circle")));
    if let Some(s) = shape_picker(ui, current, to_c32(first), true) {
        let set = set.clone();
        ed.edit(None, |d| {
            for &i in &set {
                edit::edit_dancer_everywhere(d, i, |u| u.shape = Some(s.as_str().to_owned()));
            }
        });
    }
    ui.add_space(10.0);
    if widgets::danger_button(ui, &format!("Remove {} dancers…", set.len())).clicked() {
        ed.modal = Some(editor::EdModal::RemoveDancers(set.clone()));
    }
}

fn prop(ed: &mut Editor, ui: &mut Ui, i: usize) {
    let f = ed.cur();
    let Some(p) = edit::props(&ed.doc, f).get(i).cloned() else { return };
    let can = ed.can_edit;
    header(ui, &format!("Prop · {}", geom::prop_label(p.kind())), &format!("In formation {}", f + 1));
    let key = |k: &str| format!("prop{f}-{i}-{k}");
    let mut np = p.clone();
    let mut changed: Option<String> = None;
    egui::Grid::new("prop_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label("Type");
        ui.add_enabled_ui(can, |ui| {
            egui::ComboBox::from_id_salt("prop_type").selected_text(geom::prop_label(p.kind())).show_ui(ui, |ui| {
                for &k in PropType::ALL {
                    if ui.add(egui::Button::selectable(p.kind() == k, geom::prop_label(k))).clicked() {
                        np.prop_type = k.as_str().to_owned();
                        changed = Some(key("type"));
                    }
                }
            });
        });
        ui.end_row();
        ui.label("Width (x)");
        if ui.add_enabled(can, size_value(&mut np.width)).changed() {
            changed = Some(key("w"));
        }
        ui.end_row();
        ui.label("Length (y)");
        if ui.add_enabled(can, size_value(&mut np.length)).changed() {
            changed = Some(key("l"));
        }
        ui.end_row();
        ui.label("Height");
        if ui
            .add_enabled(can, egui::DragValue::new(&mut np.height).speed(0.05).range(0.0..=100.0).max_decimals(2))
            .on_hover_text("Used by the app's 3D view")
            .changed()
        {
            changed = Some(key("h"));
        }
        ui.end_row();
        ui.label("Rotation");
        let mut deg = p.rotation_angle.to_degrees();
        if ui
            .add_enabled(can, egui::DragValue::new(&mut deg).speed(1.0).range(-180.0..=180.0).max_decimals(1).suffix("°"))
            .changed()
        {
            np.rotation_angle = geom::normalize_angle(deg.to_radians());
            changed = Some(key("rot"));
        }
        ui.end_row();
        ui.label("Position");
        ui.horizontal(|ui| {
            let b = geom::drawable_bounds(&ed.doc);
            let rx = ui.add_enabled(can, egui::DragValue::new(&mut np.position.x).speed(0.05).range(b.min.x..=b.max.x).max_decimals(2));
            let ry = ui.add_enabled(can, egui::DragValue::new(&mut np.position.y).speed(0.05).range(b.min.y..=b.max.y).max_decimals(2));
            if rx.changed() || ry.changed() {
                changed = Some(key("pos"));
            }
        });
        ui.end_row();
        ui.label("Colour");
        let mut c = to_c32(p.color);
        ui.add_enabled_ui(can, |ui| {
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, Alpha::Opaque).changed() {
                np.color = from_c32(c);
                changed = Some(key("color"));
            }
        });
        ui.end_row();
        ui.label("Opacity");
        if ui.add_enabled(can, egui::Slider::new(&mut np.alpha, 0.0..=1.0).fixed_decimals(2)).changed() {
            changed = Some(key("alpha"));
        }
        ui.end_row();
    });
    if let Some(k) = changed {
        ed.edit(Some(&k), |d| {
            if let Some(x) = edit::prop_mut(d, f, i) {
                *x = np;
            }
        });
    }
    if can {
        ui.label(
            RichText::new("Drag to move, drag the corners to resize, the round knob to rotate (snaps to 15°; hold Shift for free rotation).")
                .small()
                .color(theme::TEXT_FAINT),
        );
        ui.add_space(8.0);
        if widgets::danger_button(ui, "Delete prop").clicked() {
            ed.delete_selection();
        }
    }
}

fn size_value(v: &mut f64) -> egui::DragValue<'_> {
    egui::DragValue::new(v).speed(0.05).range(geom::MIN_PROP_SIZE..=200.0).max_decimals(2)
}

/// The view toggles (shared by writers and viewers).
fn view_toggles(ui: &mut Ui, s: &mut Settings) -> bool {
    let mut ch = false;
    let mut cb = |ui: &mut Ui, v: &mut bool, label: &str, tip: &str| {
        let r = ui.checkbox(v, label);
        let r = if tip.is_empty() { r } else { r.on_hover_text(tip) };
        ch |= r.changed();
    };
    cb(ui, &mut s.is_grid_shown, "Grid", "");
    cb(ui, &mut s.is_subdivisions_shown, "Half-cell lines", "");
    cb(ui, &mut s.is_distance_marks_shown, "Distance marks", "Cell numbers along the front edge, counted from the centre line");
    cb(ui, &mut s.is_scene_directions_shown, "Stage directions", "AUDIENCE / BACKSTAGE / STAGE LEFT / STAGE RIGHT");
    cb(ui, &mut s.is_scene_directions_inverted, "View from backstage", "Rotate the stage 180°");
    cb(ui, &mut s.is_backstage_shown, "Backstage", "Turn the outer cells of the grid into backstage (always on when the stage settings set a backstage)");
    cb(ui, &mut s.is_dancer_names_shown, "Dancer names", "");
    cb(ui, &mut s.is_dancer_indexes_shown, "Dancer numbers", "Small numbers next to dancers whose label is not their number");
    cb(ui, &mut s.is_ghost_dancers_shown, "Previous formation (ghosts)", "");
    cb(ui, &mut s.is_future_dancers_shown, "Next formation (outlines)", "");
    cb(ui, &mut s.is_transition_paths_shown, "Transition paths", "Where each dancer comes from");
    cb(ui, &mut s.is_additional_indicators_shown, "Additional indicators", "Coordinates while dragging dancers");
    ch
}

#[derive(PartialEq, Clone, Copy)]
enum BgChoice {
    None,
    Builtin(DefaultStageBackground),
    Custom,
}

fn bg_label(b: BgChoice) -> &'static str {
    match b {
        BgChoice::None => "None",
        BgChoice::Builtin(d) => paint::builtin_label(d),
        BgChoice::Custom => "Custom image",
    }
}

fn settings(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    let can = ed.can_edit;
    header(ui, "Choreography", if can { "Select a dancer or prop on the stage to edit it." } else { "" });

    if can {
        let mut name = ed.doc.name.clone();
        ui.label("Name");
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).changed() {
            ed.edit(Some("name"), |d| d.name = name);
        }
        ui.label("Description");
        let mut desc = ed.doc.description_text.clone().unwrap_or_default();
        if ui.add(egui::TextEdit::multiline(&mut desc).desired_rows(3).desired_width(f32::INFINITY)).changed() {
            ed.edit(Some("desc"), |d| d.description_text = (!desc.is_empty()).then_some(desc));
        }
    } else {
        if let Some(d) = ed.doc.description_text.as_deref().filter(|d| !d.trim().is_empty()) {
            ui.label(d);
        }
        ui.label(
            RichText::new(format!(
                "{} formations · {} dancers · {} × {} stage",
                ed.doc.formations.len(),
                ed.doc.dancer_count(),
                ed.doc.grid_size.rows,
                ed.doc.grid_size.columns
            ))
            .color(theme::TEXT_WEAK),
        );
    }

    widgets::section(ui, if can { "View" } else { "View (only on this screen)" });
    let mut s = ed.doc.settings.clone();
    if view_toggles(ui, &mut s) {
        let refit = s.is_backstage_shown != ed.doc.settings.is_backstage_shown;
        ed.set_settings(s, None);
        if refit {
            ed.need_fit = true;
        }
    }

    widgets::section(ui, "Dancers");
    let mut s = ed.doc.settings.clone();
    egui::Grid::new("dancer_settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label("Size");
        if ui.add(egui::Slider::new(&mut s.dancer_size_multiplier, 0.3..=1.5).fixed_decimals(2)).changed() {
            ed.set_settings(s.clone(), Some("size"));
        }
        ui.end_row();
        ui.label("Label size");
        if ui.add(egui::Slider::new(&mut s.dancer_label_size_multiplier, 0.5..=2.0).fixed_decimals(2)).changed() {
            ed.set_settings(s.clone(), Some("labelsize"));
        }
        ui.end_row();
        ui.label("Label");
        let style = s.fill_style();
        let label = |f: DancerFillStyle| match f {
            DancerFillStyle::Indexed => "Numbers",
            DancerFillStyle::ShortNamed => "Short names",
            DancerFillStyle::Solid => "None (solid)",
        };
        egui::ComboBox::from_id_salt("fill_style").selected_text(label(style)).show_ui(ui, |ui| {
            for &f in DancerFillStyle::ALL {
                if ui.add(egui::Button::selectable(style == f, label(f))).clicked() {
                    s.dancer_fill_style = f.as_str().to_owned();
                    ed.set_settings(s.clone(), None);
                }
            }
        });
        ui.end_row();
    });

    widgets::section(ui, "Playback");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.easing, Easing::Linear, "Linear");
        ui.selectable_value(&mut ed.easing, Easing::InOut, "Smooth");
    });
    ui.label(RichText::new("Transition easing, for this browser only (not saved).").small().color(theme::TEXT_FAINT));

    if !can {
        return;
    }

    widgets::section(ui, "Editing");
    let mut s = ed.doc.settings.clone();
    if ui.checkbox(&mut s.is_grid_snapping_enabled, "Snap to half cells").changed() {
        ed.set_settings(s, None);
    }

    widgets::section(ui, "Stage");
    let mut g = ed.doc.grid_size.clone();
    let mut grid_changed = None;
    egui::Grid::new("stage_settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label("Width (x)").on_hover_text("Cells across the stage (the app calls these rows)");
        if ui.add(egui::DragValue::new(&mut g.rows).range(1..=200).suffix(" cells")).changed() {
            grid_changed = Some("rows");
        }
        ui.end_row();
        ui.label("Depth (y)").on_hover_text("Cells from the back edge to the audience (the app calls these columns)");
        if ui.add(egui::DragValue::new(&mut g.columns).range(1..=200).suffix(" cells")).changed() {
            grid_changed = Some("cols");
        }
        ui.end_row();
        ui.label("Backstage sides").on_hover_text("Grid cells on each side that are backstage");
        if ui.add(egui::DragValue::new(&mut g.backstage.sides).range(0..=50)).changed() {
            grid_changed = Some("sides");
        }
        ui.end_row();
        ui.label("Backstage front").on_hover_text("Grid cells on the audience side that are backstage");
        if ui.add(egui::DragValue::new(&mut g.backstage.front).range(0..=50)).changed() {
            grid_changed = Some("front");
        }
        ui.end_row();
        ui.label("Backstage back").on_hover_text("Grid cells at the back that are backstage");
        if ui.add(egui::DragValue::new(&mut g.backstage.back).range(0..=50)).changed() {
            grid_changed = Some("back");
        }
        ui.end_row();
    });
    if let Some(k) = grid_changed {
        ed.edit(Some(k), |d| d.grid_size = g);
        ed.need_fit = true;
    }

    background(ed, app, ui);
}

fn background(ed: &mut Editor, app: &mut App, ui: &mut Ui) {
    widgets::section(ui, "Background");
    let bg = ed.doc.stage_background.clone();
    let builtin = bg.as_ref().and_then(|b| b.file.as_ref()).and_then(|f| f.standard_background_value.as_deref());
    let current = if ed.bg_data.is_some() {
        BgChoice::Custom
    } else if let Some(name) = builtin {
        BgChoice::Builtin(DefaultStageBackground::parse(name))
    } else {
        BgChoice::None
    };
    let missing_custom = current == BgChoice::None
        && bg.as_ref().and_then(|b| b.file.as_ref()).and_then(|f| f.file_path.as_deref()).is_some_and(|p| !p.is_empty());
    let mut pick = None;
    egui::ComboBox::from_id_salt("bg_choice").width(200.0).selected_text(bg_label(current)).show_ui(ui, |ui| {
        let mut opts = vec![BgChoice::None];
        opts.extend(DefaultStageBackground::ALL.iter().map(|&d| BgChoice::Builtin(d)));
        if ed.bg_data.is_some() {
            opts.push(BgChoice::Custom);
        }
        for o in opts {
            if ui.add(egui::Button::selectable(current == o, bg_label(o))).clicked() && current != o {
                pick = Some(o);
            }
        }
    });
    if missing_custom {
        ui.label(RichText::new("The phone's custom image is not in this file.").small().color(theme::WARN));
    }
    match pick {
        Some(BgChoice::None) => ed.set_background(None, None),
        Some(BgChoice::Builtin(d)) => {
            let old = bg.clone();
            let b = StageBackground {
                file: Some(StageBackgroundFile {
                    standard_background_value: Some(d.as_str().to_owned()),
                    ..Default::default()
                }),
                aspect_ratio: old.as_ref().map_or_else(|| "scaleToFit".into(), |b| b.aspect_ratio.clone()),
                alpha: old.as_ref().map_or(1.0, |b| b.alpha),
                is_backstage_separated: old.as_ref().is_some_and(|b| b.is_backstage_separated),
                extra: old.map(|b| b.extra).unwrap_or_default(),
            };
            ed.set_background(Some(b), None);
        }
        _ => {}
    }
    if ui.button("Choose image…").on_hover_text("PNG or JPEG, stored inside the choreography").clicked() {
        editor::pick_background(ed, app);
    }
    if let Some(b) = ed.doc.stage_background.clone() {
        let mut nb = b.clone();
        let mut key = None;
        egui::Grid::new("bg_settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("Opacity");
            if ui.add(egui::Slider::new(&mut nb.alpha, 0.0..=1.0).fixed_decimals(2)).changed() {
                key = Some(Some("bgalpha"));
            }
            ui.end_row();
            if ed.bg_data.is_some() {
                ui.label("Fit");
                let cur = StageAspectRatio::parse(&b.aspect_ratio);
                let label = |a: StageAspectRatio| match a {
                    StageAspectRatio::ScaleToFit => "Fit inside",
                    StageAspectRatio::ScaleToFill => "Fill (crop)",
                    StageAspectRatio::StretchToFill => "Stretch",
                };
                egui::ComboBox::from_id_salt("bg_aspect").selected_text(label(cur)).show_ui(ui, |ui| {
                    for &a in StageAspectRatio::ALL {
                        if ui.add(egui::Button::selectable(cur == a, label(a))).clicked() {
                            nb.aspect_ratio = a.as_str().to_owned();
                            key = Some(None);
                        }
                    }
                });
                ui.end_row();
            }
        });
        if ui
            .checkbox(&mut nb.is_backstage_separated, "Keep backstage separate")
            .on_hover_text("Draw the background on the stage only, even when the backstage is shown")
            .changed()
        {
            key = Some(None);
        }
        if let Some(k) = key {
            ed.edit(k, |d| d.stage_background = Some(nb));
        }
    }
}
