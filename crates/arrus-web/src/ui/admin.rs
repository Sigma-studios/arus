//! Admin: teams and their members, and where storage alerts go.

use arrus_api::{
    AlertSettings, CreateMember, CreateTeam, Member, Team, TeamId, UpdateAlertSettings, UpdateMember, UpdateTeam,
};
use egui::{Align, Frame, Layout, Margin, RichText, ScrollArea, Ui};

use super::widgets::{self, Choice};
use super::{App, net};
use crate::{theme, util};

#[derive(Default)]
pub struct Admin {
    pub members: Vec<Member>,
    pub members_for: Option<TeamId>,
    new_team: String,
    rename: Option<(TeamId, String)>,
    delete_team: Option<(TeamId, String, String)>,
    new_member: String,
    new_member_write: bool,
    created: Option<Member>,
    confirm_delete_member: Option<Member>,
    confirm_regen: Option<Member>,
    alerts: Option<AlertSettings>,
    webhook_input: String,
    mention_input: String,
    alerts_busy: bool,
    confirm_remove_webhook: bool,
}

pub fn load_alerts(app: &mut App) {
    app.net.spawn(async move {
        let r = net::get::<AlertSettings>("/api/settings/alerts").await;
        move |app: &mut App| match r {
            Ok(s) => {
                app.admin.mention_input = s.mention.clone();
                app.admin.alerts = Some(s);
            }
            Err(e) => app.fail(e),
        }
    });
}

fn save_alerts(app: &mut App, body: UpdateAlertSettings) {
    app.admin.alerts_busy = true;
    let webhook_changed = body.webhook.is_some();
    app.net.spawn(async move {
        let r = net::send::<_, AlertSettings>("PUT", "/api/settings/alerts", &body).await;
        move |app: &mut App| {
            app.admin.alerts_busy = false;
            match r {
                Ok(s) => {
                    if webhook_changed {
                        app.admin.webhook_input.clear();
                        app.toast(if s.webhook_set { "Webhook saved. Send a test to check it." } else { "Webhook removed" });
                    } else {
                        app.toast("Saved");
                    }
                    app.admin.mention_input = s.mention.clone();
                    app.admin.alerts = Some(s);
                }
                Err(e) => app.fail(e),
            }
        }
    });
}

fn test_alerts(app: &mut App) {
    app.admin.alerts_busy = true;
    app.net.spawn(async move {
        let r = net::send_empty("POST", "/api/settings/alerts/test").await;
        move |app: &mut App| {
            app.admin.alerts_busy = false;
            match r {
                Ok(()) => app.toast("Test message sent: check the Discord channel."),
                Err(e) => app.fail(e),
            }
        }
    });
}

pub fn load_members(app: &mut App) {
    let Some(team) = app.team else {
        app.admin.members.clear();
        app.admin.members_for = None;
        return;
    };
    app.net.spawn(async move {
        let r = net::get::<Vec<Member>>(&format!("/api/teams/{team}/members")).await;
        move |app: &mut App| {
            if app.team != Some(team) {
                return;
            }
            match r {
                Ok(mut m) => {
                    m.sort_by_key(|m| m.username.to_lowercase());
                    app.admin.members = m;
                    app.admin.members_for = Some(team);
                }
                Err(e) => app.fail(e),
            }
        }
    });
}

fn create_team(app: &mut App, name: String) {
    app.net.spawn(async move {
        let r = net::send::<_, Team>("POST", "/api/teams", &CreateTeam { name }).await;
        move |app: &mut App| match r {
            Ok(t) => {
                app.admin.new_team.clear();
                app.toast(format!("Team “{}” created", t.name));
                let id = t.id;
                app.teams.push(t);
                app.set_team(Some(id));
            }
            Err(e) => app.fail(e),
        }
    });
}

fn rename_team(app: &mut App, id: TeamId, name: String) {
    app.net.spawn(async move {
        let r = net::send::<_, Team>("PATCH", &format!("/api/teams/{id}"), &UpdateTeam { name }).await;
        move |app: &mut App| match r {
            Ok(t) => {
                if let Some(x) = app.teams.iter_mut().find(|x| x.id == t.id) {
                    *x = t;
                }
            }
            Err(e) => app.fail(e),
        }
    });
}

fn delete_team(app: &mut App, id: TeamId) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/teams/{id}")).await;
        move |app: &mut App| match r {
            Ok(()) => {
                app.teams.retain(|t| t.id != id);
                app.toast("Team deleted");
                if app.team == Some(id) {
                    let first = app.teams.first().map(|t| t.id);
                    app.set_team(first);
                }
                app.refresh_storage();
            }
            Err(e) => app.fail(e),
        }
    });
}

fn add_member(app: &mut App, team: TeamId, username: String, can_write: bool) {
    app.net.spawn(async move {
        let r = net::send::<_, Member>("POST", &format!("/api/teams/{team}/members"), &CreateMember { username, can_write })
            .await;
        move |app: &mut App| match r {
            Ok(m) => {
                app.admin.new_member.clear();
                app.admin.created = Some(m.clone());
                if app.team == Some(m.team_id) {
                    app.admin.members.push(m);
                }
                app.load_teams();
            }
            Err(net::ApiError::Http { code: Some(c), .. }) if c == "username_taken" => {
                app.toast_error("That username is already taken.")
            }
            Err(e) => app.fail(e),
        }
    });
}

fn update_member(app: &mut App, id: i64, body: UpdateMember) {
    let regen = body.regenerate_password;
    app.net.spawn(async move {
        let r = net::send::<_, Member>("PATCH", &format!("/api/members/{id}"), &body).await;
        move |app: &mut App| match r {
            Ok(m) => {
                if regen {
                    app.toast(format!("New password for {}: {}", m.username, m.password));
                }
                if let Some(x) = app.admin.members.iter_mut().find(|x| x.id == m.id) {
                    *x = m;
                }
            }
            Err(e) => {
                app.fail(e);
                load_members(app);
            }
        }
    });
}

fn delete_member(app: &mut App, id: i64) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/members/{id}")).await;
        move |app: &mut App| match r {
            Ok(()) => {
                app.admin.members.retain(|m| m.id != id);
                app.load_teams();
            }
            Err(e) => app.fail(e),
        }
    });
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(28, 18)))
        .show(ui, |ui| {
            ui.heading("Teams & members");
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(310.0);
                    teams_card(app, ui);
                    ui.add_space(14.0);
                    alerts_card(app, ui);
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    members_card(app, ui);
                });
            });
        });
    dialogs(app, &ctx);
}

fn teams_card(app: &mut App, ui: &mut Ui) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section(ui, "Teams");
        let mut pick = None;
        ScrollArea::vertical().max_height(420.0).id_salt("teams_list").show(ui, |ui| {
            for t in &app.teams {
                let selected = app.team == Some(t.id);
                let r = ui.add(
                    egui::Button::selectable(
                        selected,
                        RichText::new(&t.name).strong().size(15.0),
                    )
                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                );
                ui.label(
                    RichText::new(format!(
                        "{} members · {} choreographies · {}",
                        t.member_count,
                        t.choreography_count,
                        util::fmt_bytes(t.bytes_used)
                    ))
                    .small()
                    .color(theme::TEXT_WEAK),
                );
                ui.add_space(4.0);
                if r.clicked() {
                    pick = Some(t.id);
                }
            }
            if app.teams.is_empty() {
                ui.label(RichText::new("No teams yet.").color(theme::TEXT_WEAK));
            }
        });
        if let Some(id) = pick {
            app.set_team(Some(id));
        }
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.admin.new_team).hint_text("New team name").desired_width(180.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let name = app.admin.new_team.trim().to_owned();
            if (ui.add_enabled(!name.is_empty(), egui::Button::new("Create")).clicked() || enter) && !name.is_empty() {
                create_team(app, name);
            }
        });
        if let Some(t) = app.team.and_then(|id| app.teams.iter().find(|t| t.id == id)).cloned() {
            ui.horizontal(|ui| {
                if ui.button("Rename…").clicked() {
                    app.admin.rename = Some((t.id, t.name.clone()));
                }
                if ui.button(RichText::new("Delete team…").color(theme::ERROR)).clicked() {
                    app.admin.delete_team = Some((t.id, t.name.clone(), String::new()));
                }
            });
        }
    });
}

fn alerts_card(app: &mut App, ui: &mut Ui) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section(ui, "Storage alerts");
        let limit = app.storage.as_ref().map(|s| util::fmt_bytes(s.limit)).unwrap_or_else(|| "the limit".into());
        ui.label(
            RichText::new(format!(
                "Posted to a Discord channel at 80 % and 95 % of {limit}, and when a save or upload is refused because storage is full."
            ))
            .small()
            .color(theme::TEXT_WEAK),
        );
        ui.add_space(8.0);
        let Some(settings) = app.admin.alerts.clone() else {
            ui.spinner();
            return;
        };
        let busy = app.admin.alerts_busy;

        if let Some(hint) = settings.webhook_hint.as_deref().filter(|_| settings.webhook_set) {
            ui.horizontal(|ui| {
                widgets::badge(ui, "ON", theme::OK);
                ui.label(RichText::new(hint).monospace().small());
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Send test")).clicked() {
                    test_alerts(app);
                }
                if ui.add_enabled(!busy, egui::Button::new(RichText::new("Remove").color(theme::ERROR))).clicked() {
                    app.admin.confirm_remove_webhook = true;
                }
            });
        } else {
            ui.horizontal(|ui| {
                widgets::badge(ui, "OFF", theme::WARN);
                ui.label(RichText::new("No webhook: alerts only reach the server log.").small().color(theme::TEXT_WEAK));
            });
        }

        ui.add_space(8.0);
        ui.label(RichText::new(if settings.webhook_set { "Replace the webhook" } else { "Webhook URL" }).small().strong());
        ui.add(
            egui::TextEdit::singleline(&mut app.admin.webhook_input)
                .hint_text("https://discord.com/api/webhooks/…")
                .password(true)
                .desired_width(f32::INFINITY),
        );
        let url = app.admin.webhook_input.trim().to_owned();
        if ui.add_enabled(!busy && !url.is_empty(), egui::Button::new("Save webhook")).clicked() {
            save_alerts(app, UpdateAlertSettings { webhook: Some(url), mention: None });
        }
        ui.label(
            RichText::new("In Discord: Edit channel → Integrations → Webhooks → New Webhook → Copy Webhook URL.")
                .small()
                .color(theme::TEXT_FAINT),
        );

        ui.add_space(8.0);
        ui.label(RichText::new("Tag").small().strong());
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.admin.mention_input)
                    .hint_text("nobody")
                    .desired_width(ui.available_width() - 60.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let changed = app.admin.mention_input.trim() != settings.mention;
            if (ui.add_enabled(!busy && changed, egui::Button::new("Save")).clicked() || (enter && changed)) && !busy {
                let mention = app.admin.mention_input.trim().to_owned();
                save_alerts(app, UpdateAlertSettings { webhook: None, mention: Some(mention) });
            }
        });
        ui.label(
            RichText::new("@here, @everyone, user:<id> or role:<id> (ids from Discord's developer mode). Empty: tag nobody.")
                .small()
                .color(theme::TEXT_FAINT),
        );
    });
}

fn members_card(app: &mut App, ui: &mut Ui) {
    let Some(team) = app.team else {
        return;
    };
    let team_name = app.team_name().unwrap_or_default();
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section(ui, &format!("Members of {team_name}"));
        if let Some(m) = app.admin.created.clone() {
            Frame::new()
                .fill(theme::OK.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0, theme::OK.gamma_multiply(0.6)))
                .corner_radius(8)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("Created {} — password:", m.username));
                        ui.label(RichText::new(&m.password).monospace().strong().size(15.0));
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(format!("{} / {}", m.username, m.password));
                            app.toast("Copied");
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                app.admin.created = None;
                            }
                        });
                    });
                    ui.label(
                        RichText::new("Give these to the member. They log in at this address; the password cannot be changed by them.")
                            .small()
                            .color(theme::TEXT_WEAK),
                    );
                });
            ui.add_space(8.0);
        }

        let mut updates: Vec<(i64, UpdateMember)> = Vec::new();
        egui::Grid::new("members_grid").striped(true).spacing([18.0, 10.0]).show(ui, |ui| {
            for h in ["Username", "Password", "Can edit", ""] {
                ui.label(RichText::new(h).small().strong().color(theme::TEXT_FAINT));
            }
            ui.end_row();
            for m in &app.admin.members {
                ui.label(RichText::new(&m.username).strong());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&m.password).monospace());
                    if ui.small_button("Copy").on_hover_text("Copy the password").clicked() {
                        ui.ctx().copy_text(m.password.clone());
                    }
                });
                let mut w = m.can_write;
                if ui.checkbox(&mut w, "").on_hover_text("Write access: create, edit, import and delete").changed() {
                    updates.push((m.id, UpdateMember { can_write: Some(w), ..Default::default() }));
                }
                ui.horizontal(|ui| {
                    if ui.button("New password").clicked() {
                        app.admin.confirm_regen = Some(m.clone());
                    }
                    if ui.button(RichText::new("Delete").color(theme::ERROR)).clicked() {
                        app.admin.confirm_delete_member = Some(m.clone());
                    }
                });
                ui.end_row();
            }
        });
        for (id, u) in updates {
            if let (Some(w), Some(m)) = (u.can_write, app.admin.members.iter_mut().find(|m| m.id == id)) {
                m.can_write = w;
            }
            update_member(app, id, u);
        }
        if app.admin.members.is_empty() && app.admin.members_for == Some(team) {
            ui.label(RichText::new("No members yet.").color(theme::TEXT_WEAK));
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.admin.new_member).hint_text("Username").desired_width(180.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.checkbox(&mut app.admin.new_member_write, "Can edit");
            let name = app.admin.new_member.trim().to_owned();
            if (ui.add_enabled(!name.is_empty(), egui::Button::new("Add member")).clicked() || enter) && !name.is_empty() {
                let w = app.admin.new_member_write;
                add_member(app, team, name, w);
            }
        });
        ui.label(RichText::new("A password is generated for each member.").small().color(theme::TEXT_WEAK));
    });
}

fn dialogs(app: &mut App, ctx: &egui::Context) {
    if let Some((id, name)) = &mut app.admin.rename {
        let id = *id;
        let valid = !name.trim().is_empty();
        let c = widgets::dialog(ctx, "rename_team", "Rename team", "Rename", false, valid, |ui| {
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        });
        let name = name.trim().to_owned();
        match c {
            Some(Choice::Confirm) => {
                app.admin.rename = None;
                rename_team(app, id, name);
            }
            Some(Choice::Cancel) => app.admin.rename = None,
            None => {}
        }
    }
    if let Some((id, name, typed)) = &mut app.admin.delete_team {
        let id = *id;
        let ok = typed.trim() == name.as_str();
        let n = name.clone();
        let c = widgets::dialog(ctx, "delete_team", "Delete team?", "Delete team", true, ok, |ui| {
            ui.label(format!(
                "This deletes “{n}”, all its members (their logins stop working), all its choreographies and music. This cannot be undone."
            ));
            ui.add_space(6.0);
            ui.label(format!("Type the team name ({n}) to confirm:"));
            ui.add(egui::TextEdit::singleline(typed).desired_width(f32::INFINITY));
        });
        match c {
            Some(Choice::Confirm) => {
                app.admin.delete_team = None;
                delete_team(app, id);
            }
            Some(Choice::Cancel) => app.admin.delete_team = None,
            None => {}
        }
    }
    if let Some(m) = app.admin.confirm_regen.clone() {
        match widgets::dialog(ctx, "regen_pw", "Generate a new password?", "Generate", false, true, |ui| {
            ui.label(format!("{} will be logged out everywhere and need the new password.", m.username));
        }) {
            Some(Choice::Confirm) => {
                app.admin.confirm_regen = None;
                update_member(app, m.id, UpdateMember { regenerate_password: true, ..Default::default() });
            }
            Some(Choice::Cancel) => app.admin.confirm_regen = None,
            None => {}
        }
    }
    if app.admin.confirm_remove_webhook {
        match widgets::dialog(ctx, "remove_webhook", "Remove the webhook?", "Remove", true, true, |ui| {
            ui.label("Storage alerts will no longer be posted to Discord.");
        }) {
            Some(Choice::Confirm) => {
                app.admin.confirm_remove_webhook = false;
                save_alerts(app, UpdateAlertSettings { webhook: Some(String::new()), mention: None });
            }
            Some(Choice::Cancel) => app.admin.confirm_remove_webhook = false,
            None => {}
        }
    }
    if let Some(m) = app.admin.confirm_delete_member.clone() {
        match widgets::dialog(ctx, "del_member", "Delete member?", "Delete", true, true, |ui| {
            ui.label(format!("{} will lose access immediately. Their choreographies stay with the team.", m.username));
        }) {
            Some(Choice::Confirm) => {
                app.admin.confirm_delete_member = None;
                delete_member(app, m.id);
            }
            Some(Choice::Cancel) => app.admin.confirm_delete_member = None,
            None => {}
        }
    }
}
