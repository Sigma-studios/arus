//! Premium ended while the account held more than the free plan allows: its
//! teams are read-only until it picks the one team, and the choreographies
//! in it, that stay editable. A bar says so; the dialog makes the choice
//! (`POST /api/me/keep`, once).

use arrus_api::{ChoreographyId, ChoreographySummary, FREE_CHOREOGRAPHIES, KeepChoice, Me, TeamId, TeamRole};
use egui::{Frame, Margin, RichText, Ui};

use super::widgets::{self, Choice};
use super::{App, library, net};
use crate::{theme, web};

#[derive(Default)]
pub struct KeepUi {
    open: bool,
    /// Opened by itself once per page load.
    shown: bool,
    team: Option<TeamId>,
    /// The choreographies of `team` (once fetched).
    items: Option<Vec<ChoreographySummary>>,
    picked: Vec<ChoreographyId>,
    busy: bool,
}

fn needed(app: &App) -> bool {
    app.me.as_ref().is_some_and(|m| m.must_choose)
}

fn owned(app: &App) -> Vec<(TeamId, String, u32)> {
    app.my_teams().iter().filter(|t| t.role == TeamRole::Owner).map(|t| (t.id, t.name.clone(), t.choreography_count)).collect()
}

pub fn open(app: &mut App) {
    app.keep = KeepUi { open: true, shown: true, ..Default::default() };
    // Preselect when there is only one team to keep.
    if let [(id, _, _)] = owned(app).as_slice() {
        pick_team(app, *id);
    }
}

fn pick_team(app: &mut App, id: TeamId) {
    app.keep.team = Some(id);
    app.keep.items = None;
    app.keep.picked.clear();
    app.net.spawn(async move {
        let r = net::get::<Vec<ChoreographySummary>>(&format!("/api/choreographies?team={id}")).await;
        move |app: &mut App| {
            if app.keep.team != Some(id) {
                return;
            }
            match r {
                Ok(mut items) => {
                    items.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
                    // Few enough: all of them, nothing to pick.
                    if items.len() <= FREE_CHOREOGRAPHIES as usize {
                        app.keep.picked = items.iter().map(|c| c.id).collect();
                    }
                    app.keep.items = Some(items);
                }
                Err(e) => app.fail(e),
            }
        }
    });
}

fn submit(app: &mut App) {
    let body = KeepChoice { team_id: app.keep.team, choreographies: app.keep.picked.clone() };
    app.keep.busy = true;
    app.net.spawn(async move {
        let r = net::send::<_, Me>("POST", "/api/me/keep", &body).await;
        move |app: &mut App| {
            app.keep.busy = false;
            match r {
                Ok(me) => {
                    app.me = Some(me);
                    app.keep.open = false;
                    app.toast("Saved: that team stays editable");
                    if body.team_id.is_some() {
                        app.set_team(body.team_id);
                    }
                    library::reload(app);
                }
                Err(e) => {
                    app.keep.open = false;
                    app.reload_me();
                    app.fail(e);
                }
            }
        }
    });
}

/// "Premium has ended …: choose": above the screens while the choice is open.
pub fn bar(app: &mut App, ui: &mut Ui) {
    if !needed(app) {
        return;
    }
    if !app.keep.shown {
        open(app);
    }
    let compact = app.compact;
    let mut choose = false;
    egui::Panel::top("keep_bar")
        .frame(Frame::new().fill(theme::surface()).inner_margin(Margin::symmetric(14, 8)))
        .show_separator_line(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("🔒").color(theme::WARN));
                ui.label(RichText::new(if compact { "Your teams are read-only." } else { "Premium has ended: your teams are read-only." }).strong());
                if !compact {
                    ui.label(
                        RichText::new("Choose the team and the choreographies you keep editing on the free plan.")
                            .small()
                            .color(theme::TEXT_WEAK),
                    );
                }
                ui.add_space(6.0);
                choose = widgets::primary_button(ui, "Choose…").clicked();
            });
        });
    if choose {
        open(app);
    }
}

pub fn dialog(app: &mut App, ctx: &egui::Context) {
    if !app.keep.open {
        return;
    }
    if !needed(app) {
        app.keep.open = false;
        return;
    }
    let teams = owned(app);
    let want = app.keep.items.as_ref().map(|i| i.len().min(FREE_CHOREOGRAPHIES as usize));
    let ready = !app.keep.busy
        && match app.keep.team {
            None => teams.is_empty(),
            Some(_) => want.is_some_and(|w| app.keep.picked.len() == w),
        };
    let mut team_pick = None;
    let k = &mut app.keep;
    let c = widgets::dialog(ctx, "keep_choice", "Choose what you keep editing", "Keep these", false, ready, |ui| {
        ui.label(format!(
            "Premium has ended. The free plan includes 1 team with up to {FREE_CHOREOGRAPHIES} choreographies. \
             Choose them below."
        ));
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Everything else stays readable: you and your team can still open, export (.arrus, PDF) and \
                 delete it, but no longer change it.",
            )
            .small()
            .color(theme::TEXT_WEAK),
        );
        if !web::in_app() {
            ui.label(
                RichText::new("With Premium again, everything becomes editable again.").small().color(theme::TEXT_WEAK),
            );
        }
        ui.label(RichText::new("You choose once: this can't be changed later.").small().color(theme::WARN));
        ui.add_space(10.0);
        if teams.is_empty() {
            ui.label("You own no team any more: nothing to choose.");
            return;
        }
        ui.label(RichText::new("Team").strong());
        for (id, name, n) in &teams {
            let text = format!("{name}  ({n} choreograph{})", if *n == 1 { "y" } else { "ies" });
            if ui.radio(k.team == Some(*id), text).clicked() && k.team != Some(*id) {
                team_pick = Some(*id);
            }
        }
        let Some(_) = k.team else { return };
        ui.add_space(10.0);
        let Some(items) = &k.items else {
            ui.spinner();
            return;
        };
        let want = items.len().min(FREE_CHOREOGRAPHIES as usize);
        if items.is_empty() {
            ui.label(RichText::new("This team has no choreographies.").color(theme::TEXT_WEAK));
            return;
        }
        if items.len() <= FREE_CHOREOGRAPHIES as usize {
            ui.label(RichText::new("Its choreographies, all kept:").strong());
        } else {
            ui.label(RichText::new(format!("Choreographies ({} of {want} chosen)", k.picked.len())).strong());
        }
        let all = items.len() <= FREE_CHOREOGRAPHIES as usize;
        for c in items {
            let mut on = k.picked.contains(&c.id);
            let full = k.picked.len() >= want;
            let text = format!("{}  · {} formations · {} dancers", c.name, c.formation_count, c.dancer_count);
            let r = ui.add_enabled(!all && (on || !full), egui::Checkbox::new(&mut on, text));
            if r.changed() {
                if on {
                    k.picked.push(c.id);
                } else {
                    k.picked.retain(|p| *p != c.id);
                }
            }
        }
    });
    if let Some(id) = team_pick {
        pick_team(app, id);
    }
    match c {
        Some(Choice::Confirm) => submit(app),
        Some(Choice::Cancel) => app.keep.open = false,
        None => {}
    }
}
