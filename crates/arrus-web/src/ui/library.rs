//! The team's choreographies: open, export, delete, new, import.

use arrus_api::{ChoreographyId, ChoreographySummary, CreateChoreography, ImportResult, TeamId};
use egui::{Align, Frame, Layout, Margin, RichText, ScrollArea, Ui};

use super::widgets::{self, Choice};
use super::{App, editor, net};
use crate::{edit, theme, util, web};

#[derive(Default)]
pub struct Library {
    pub items: Vec<ChoreographySummary>,
    pub loaded_for: Option<TeamId>,
    pub loading: bool,
    filter: String,
    new_dialog: Option<NewDialog>,
    confirm_delete: Option<(ChoreographyId, String)>,
    import: Option<ImportReport>,
}

struct NewDialog {
    name: String,
    dancers: u32,
    busy: bool,
}

#[derive(Default)]
struct ImportReport {
    pending: usize,
    created: Vec<String>,
    notes: Vec<String>,
    errors: Vec<String>,
}

pub fn reload(app: &mut App) {
    let url = if app.is_admin() {
        match app.team {
            Some(t) => format!("/api/choreographies?team={t}"),
            None => return,
        }
    } else {
        "/api/choreographies".to_owned()
    };
    let team = app.team;
    app.library.loading = true;
    app.net.spawn(async move {
        let r = net::get::<Vec<ChoreographySummary>>(&url).await;
        move |app: &mut App| {
            if app.team != team {
                return;
            }
            app.library.loading = false;
            match r {
                Ok(mut items) => {
                    items.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
                    app.library.items = items;
                    app.library.loaded_for = team;
                }
                Err(e) => app.fail(e),
            }
        }
    });
}

fn import_files(app: &mut App, files: Vec<web_sys::File>) {
    let Some(team) = app.team else { return };
    app.library.import = Some(ImportReport { pending: files.len(), ..Default::default() });
    for file in files {
        let name = file.name();
        app.net.spawn(async move {
            let r = net::upload::<ImportResult>(
                "POST",
                &format!("/api/teams/{team}/import"),
                file,
                "application/octet-stream".into(),
            )
            .await;
            move |app: &mut App| {
                let storage_full = matches!(r, Err(net::ApiError::StorageFull));
                if let Some(rep) = &mut app.library.import {
                    rep.pending = rep.pending.saturating_sub(1);
                    match r {
                        Ok(res) => {
                            rep.created.extend(res.created.into_iter().map(|c| c.name));
                            rep.notes.extend(res.notes);
                        }
                        Err(net::ApiError::Unauthorized) => web::navigate("/login"),
                        Err(e) => rep.errors.push(format!("{name}: {e}")),
                    }
                }
                if storage_full {
                    app.fail(net::ApiError::StorageFull);
                }
                if app.library.import.as_ref().is_some_and(|r| r.pending == 0) {
                    reload(app);
                    app.refresh_storage();
                }
            }
        });
    }
}

fn create(app: &mut App, name: String, dancers: u32) {
    let now = web::now_secs();
    let data = edit::new_choreography(&name, &web::random_uuid(), now, dancers as usize);
    let body = CreateChoreography { team_id: app.team, data };
    app.net.spawn(async move {
        let r = net::send::<_, ChoreographySummary>("POST", "/api/choreographies", &body).await;
        move |app: &mut App| {
            app.library.new_dialog = None;
            match r {
                Ok(s) => {
                    app.refresh_storage();
                    editor::open(app, s.id);
                }
                Err(e) => app.fail(e),
            }
        }
    });
}

fn delete(app: &mut App, id: ChoreographyId) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/choreographies/{id}")).await;
        move |app: &mut App| match r {
            Ok(()) => {
                app.library.items.retain(|c| c.id != id);
                app.toast("Choreography deleted");
                app.refresh_storage();
            }
            Err(e) => app.fail(e),
        }
    });
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let writer = app.can_edit();
    let compact = app.compact;
    let ctx = ui.ctx().clone();

    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Choreographies");
                if let Some(t) = app.team_name() {
                    ui.label(RichText::new(format!("· {t}")).size(18.0).color(theme::TEXT_WEAK));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if writer {
                        if widgets::primary_button(ui, "+ New").clicked() {
                            app.library.new_dialog = Some(NewDialog { name: String::new(), dancers: 8, busy: false });
                        }
                        if ui.button("Import…").on_hover_text("Import .arrus files or an .aubkp backup from the ArrangeUs app").clicked() {
                            let net = app.net.clone();
                            web::pick_files(".arrus,.aubkp", true, move |files| {
                                net.post(move |app: &mut App| import_files(app, files));
                            });
                        }
                    }
                    if ui.button("Refresh").clicked() {
                        reload(app);
                        app.refresh_storage();
                    }
                    if app.library.loading {
                        ui.spinner();
                    }
                });
            });
            ui.add_space(6.0);
            if app.is_admin() && app.team.is_none() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No team yet.").size(16.0));
                    ui.label(RichText::new("Create a team in Teams & members first.").color(theme::TEXT_WEAK));
                });
                return;
            }
            if app.library.items.len() > 6 {
                ui.add(egui::TextEdit::singleline(&mut app.library.filter).hint_text("Search…").desired_width(240.0));
                ui.add_space(6.0);
            }
            if app.library.items.is_empty() && !app.library.loading && app.library.loaded_for.is_some() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No choreographies yet").size(17.0));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(if writer {
                            "Create one with New, or import .arrus files shared from the ArrangeUs app."
                        } else {
                            "Choreographies your team adds will appear here."
                        })
                        .color(theme::TEXT_WEAK),
                    );
                });
                return;
            }

            let filter = app.library.filter.to_lowercase();
            let items: Vec<ChoreographySummary> = app
                .library
                .items
                .iter()
                .filter(|c| filter.is_empty() || c.name.to_lowercase().contains(&filter))
                .cloned()
                .collect();
            let now = web::now_secs();
            let mut open = None;
            let mut del = None;
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                if compact {
                    for c in &items {
                        widgets::card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&c.name).strong().size(16.0));
                                if c.music.is_some() {
                                    ui.label(RichText::new("♪").color(theme::ACCENT));
                                }
                            });
                            ui.label(
                                RichText::new(format!(
                                    "{} formations · {} dancers · {}",
                                    c.formation_count,
                                    c.dancer_count,
                                    updated(now, c)
                                ))
                                .small()
                                .color(theme::TEXT_WEAK),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if widgets::primary_button(ui, "Open").clicked() {
                                    open = Some(c.id);
                                }
                                if ui.button("Export").clicked() {
                                    web::download(&format!("/api/choreographies/{}/export", c.id));
                                }
                                if ui.button("PDF").clicked() {
                                    web::download(&format!("/api/choreographies/{}/pdf", c.id));
                                }
                            });
                        });
                        ui.add_space(8.0);
                    }
                    return;
                }
                egui::Grid::new("library_grid").striped(true).spacing([18.0, 10.0]).min_col_width(40.0).show(ui, |ui| {
                    for h in ["Name", "Formations", "Dancers", "Updated", "Music", "Size", ""] {
                        ui.label(RichText::new(h).small().strong().color(theme::TEXT_FAINT));
                    }
                    ui.end_row();
                    for c in &items {
                        let name = ui.add(
                            egui::Label::new(RichText::new(&c.name).strong().size(15.0)).sense(egui::Sense::click()),
                        );
                        if name.on_hover_cursor(egui::CursorIcon::PointingHand).double_clicked() {
                            open = Some(c.id);
                        }
                        ui.label(c.formation_count.to_string());
                        ui.label(c.dancer_count.to_string());
                        ui.label(RichText::new(updated(now, c)).color(theme::TEXT_WEAK))
                            .on_hover_text(web::local_datetime(c.updated_at));
                        match &c.music {
                            Some(m) => {
                                ui.label(RichText::new("♪").color(theme::ACCENT).size(16.0))
                                    .on_hover_text(format!("{} ({})", m.filename, util::fmt_bytes(m.bytes)));
                            }
                            None => {
                                ui.label(RichText::new("–").color(theme::TEXT_FAINT));
                            }
                        }
                        let size = c.size_bytes + c.music.as_ref().map_or(0, |m| m.bytes);
                        ui.label(RichText::new(util::fmt_bytes(size)).color(theme::TEXT_WEAK));
                        ui.horizontal(|ui| {
                            if widgets::primary_button(ui, "Open").clicked() {
                                open = Some(c.id);
                            }
                            if ui.button("Export").on_hover_text("Download as .arrus for the ArrangeUs app").clicked() {
                                web::download(&format!("/api/choreographies/{}/export", c.id));
                            }
                            if ui.button("PDF").on_hover_text("Download a PDF to share: one page per formation").clicked() {
                                web::download(&format!("/api/choreographies/{}/pdf", c.id));
                            }
                            if writer && ui.button(RichText::new("Delete").color(theme::ERROR)).clicked() {
                                del = Some((c.id, c.name.clone()));
                            }
                        });
                        ui.end_row();
                    }
                });
            });
            if let Some(id) = open {
                editor::open(app, id);
            }
            if del.is_some() {
                app.library.confirm_delete = del;
            }
        });

    dialogs(app, &ctx);
}

fn updated(now: i64, c: &ChoreographySummary) -> String {
    let when = util::fmt_ago(now, c.updated_at).unwrap_or_else(|| web::local_datetime(c.updated_at));
    format!("{when} by {}", c.updated_by)
}

fn dialogs(app: &mut App, ctx: &egui::Context) {
    let mut action: Option<Option<(String, u32)>> = None;
    if let Some(d) = &mut app.library.new_dialog {
        let valid = !d.name.trim().is_empty() && !d.busy;
        let mut enter = false;
        let choice = widgets::dialog(ctx, "new_choreo", "New choreography", "Create", false, valid, |ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("e.g. Spring show").desired_width(f32::INFINITY));
            if !r.has_focus() && d.name.is_empty() {
                r.request_focus();
            }
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Dancers");
                ui.add(egui::DragValue::new(&mut d.dancers).range(0..=60));
            });
            ui.label(
                RichText::new("Dancers start in centred rows on a 14 × 8 stage; you can add, remove and move them later.")
                    .small()
                    .color(theme::TEXT_WEAK),
            );
        });
        match choice {
            Some(Choice::Confirm) => action = Some(Some((d.name.trim().to_owned(), d.dancers))),
            None if enter && valid => action = Some(Some((d.name.trim().to_owned(), d.dancers))),
            Some(Choice::Cancel) => action = Some(None),
            None => {}
        }
        if let Some(Some(_)) = action {
            d.busy = true;
        }
    }
    match action {
        Some(Some((n, k))) => create(app, n, k),
        Some(None) => app.library.new_dialog = None,
        None => {}
    }

    if let Some((id, name)) = app.library.confirm_delete.clone() {
        match widgets::dialog(ctx, "del_choreo", "Delete choreography?", "Delete", true, true, |ui| {
            ui.label(format!("“{name}” and its music will be deleted for the whole team. This cannot be undone."));
            ui.label(RichText::new("Tip: Export it first to keep a copy.").small().color(theme::TEXT_WEAK));
        }) {
            Some(Choice::Confirm) => {
                app.library.confirm_delete = None;
                delete(app, id);
            }
            Some(Choice::Cancel) => app.library.confirm_delete = None,
            None => {}
        }
    }

    if let Some(rep) = &app.library.import {
        let mut close = false;
        let r = egui::Modal::new(egui::Id::unique("import_report")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading("Import");
            ui.add_space(8.0);
            if rep.pending > 0 {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Importing {} file(s)…", rep.pending));
                });
            }
            if !rep.created.is_empty() {
                ui.label(RichText::new(format!("Imported {}:", rep.created.len())).strong());
                ScrollArea::vertical().max_height(180.0).id_salt("imp_created").show(ui, |ui| {
                    for n in &rep.created {
                        ui.label(format!("• {n}"));
                    }
                });
            }
            for n in &rep.notes {
                ui.label(RichText::new(n).color(theme::TEXT_WEAK));
            }
            for e in &rep.errors {
                ui.label(RichText::new(e).color(theme::ERROR));
            }
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if rep.pending == 0 && widgets::primary_button(ui, "Done").clicked() {
                    close = true;
                }
            });
        });
        if close || (rep.pending == 0 && r.should_close()) {
            app.library.import = None;
        }
    }
}
