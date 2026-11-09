//! The current team: who is in it and as what, share links, rename, delete.
//! The owner (and the admin) manage it; others see who is in it and can leave.

use arrus_api::{AddTeamMember, CreateInvite, Invite, Team, TeamId, TeamMember, TeamRole, UpdateTeam, UpdateTeamMember};
use egui::{Frame, Margin, RichText, ScrollArea, Ui};

use super::widgets::{self, Choice};
use super::{App, Screen, net};
use crate::{theme, web};

pub struct TeamUi {
    pub members: Vec<TeamMember>,
    pub invites: Vec<Invite>,
    pub loaded_for: Option<TeamId>,
    add_nickname: String,
    add_role: TeamRole,
    link_role: TeamRole,
    rename: Option<String>,
    /// Deleting: what was typed to confirm.
    delete: Option<String>,
    remove: Option<TeamMember>,
    leave: bool,
}

impl Default for TeamUi {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            invites: Vec::new(),
            loaded_for: None,
            add_nickname: String::new(),
            add_role: TeamRole::Guest,
            link_role: TeamRole::Guest,
            rename: None,
            delete: None,
            remove: None,
            leave: false,
        }
    }
}

pub fn load(app: &mut App) {
    let Some(team) = app.team else { return };
    let manage = app.can_manage();
    app.net.spawn(async move {
        let members = net::get::<Vec<TeamMember>>(&format!("/api/teams/{team}/members")).await;
        let invites = if manage { Some(net::get::<Vec<Invite>>(&format!("/api/teams/{team}/invites")).await) } else { None };
        move |app: &mut App| {
            if app.team != Some(team) {
                return;
            }
            match members {
                Ok(m) => {
                    app.team_ui.members = m;
                    app.team_ui.loaded_for = Some(team);
                }
                Err(e) => app.fail(e),
            }
            match invites {
                Some(Ok(i)) => app.team_ui.invites = i,
                Some(Err(e)) => app.fail(e),
                None => app.team_ui.invites.clear(),
            }
        }
    });
}

fn after_change(app: &mut App) {
    load(app);
    app.reload_me();
    if app.is_admin() {
        app.load_teams();
    }
}

fn add_member(app: &mut App, team: TeamId, nickname: String, role: TeamRole) {
    app.net.spawn(async move {
        let r = net::send::<_, TeamMember>("POST", &format!("/api/teams/{team}/members"), &AddTeamMember { nickname, role }).await;
        move |app: &mut App| match r {
            Ok(m) => {
                app.team_ui.add_nickname.clear();
                app.toast(format!("{} is in the team ({})", m.nickname, m.role.label().to_lowercase()));
                after_change(app);
            }
            Err(net::ApiError::Http { code: Some(c), .. }) if c == "no_such_account" => {
                app.toast_error("No account has that nickname. They sign up first, or use a share link.")
            }
            Err(e) => app.fail(e),
        }
    });
}

fn set_role(app: &mut App, team: TeamId, account: i64, role: TeamRole) {
    app.net.spawn(async move {
        let r = net::send::<_, TeamMember>("PATCH", &format!("/api/teams/{team}/members/{account}"), &UpdateTeamMember { role }).await;
        move |app: &mut App| match r {
            Ok(_) => after_change(app),
            Err(e) => {
                app.fail(e);
                load(app);
            }
        }
    });
}

fn remove_member(app: &mut App, team: TeamId, account: i64, leaving: bool) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/teams/{team}/members/{account}")).await;
        move |app: &mut App| match r {
            Ok(()) => {
                if leaving {
                    app.toast("You left the team");
                    app.go(Screen::Library);
                }
                after_change(app);
            }
            Err(e) => app.fail(e),
        }
    });
}

fn create_invite(app: &mut App, team: TeamId, role: TeamRole) {
    app.net.spawn(async move {
        let r = net::send::<_, Invite>("POST", &format!("/api/teams/{team}/invites"), &CreateInvite { role }).await;
        move |app: &mut App| match r {
            Ok(i) => {
                app.net.ctx().copy_text(link(&i));
                app.toast("Link created and copied");
                app.team_ui.invites.push(i);
            }
            Err(e) => app.fail(e),
        }
    });
}

fn revoke(app: &mut App, token: String) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/invites/{token}")).await;
        move |app: &mut App| match r {
            Ok(()) => app.team_ui.invites.retain(|i| i.token != token),
            Err(e) => app.fail(e),
        }
    });
}

fn rename(app: &mut App, team: TeamId, name: String) {
    app.net.spawn(async move {
        let r = net::send::<_, Team>("PATCH", &format!("/api/teams/{team}"), &UpdateTeam { name }).await;
        move |app: &mut App| match r {
            Ok(_) => after_change(app),
            Err(e) => app.fail(e),
        }
    });
}

fn delete_team(app: &mut App, team: TeamId) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/teams/{team}")).await;
        move |app: &mut App| match r {
            Ok(()) => {
                app.toast("Team deleted");
                app.teams.retain(|t| t.id != team);
                if let Some(me) = &mut app.me {
                    me.teams.retain(|t| t.id != team);
                }
                let next = app.my_teams().first().map(|t| t.id).or_else(|| app.teams.first().map(|t| t.id));
                app.set_team(next);
                app.reload_me();
                app.refresh_storage();
                if app.screen == Screen::Team {
                    app.go(Screen::Library);
                }
            }
            Err(e) => app.fail(e),
        }
    });
}

fn link(i: &Invite) -> String {
    format!("{}/join/{}", web::origin(), i.token)
}

fn role_picker(ui: &mut Ui, id: &str, role: &mut TeamRole) -> bool {
    let before = *role;
    egui::ComboBox::from_id_salt(id).selected_text(role.label()).width(110.0).show_ui(ui, |ui| {
        ui.selectable_value(role, TeamRole::Guest, "Guest").on_hover_text("Views the choreographies");
        ui.selectable_value(role, TeamRole::CoOwner, "Co-owner").on_hover_text("Edits the choreographies with you");
    });
    *role != before
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let compact = app.compact;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(720.0);
                card(app, ui);
            });
        });
    dialogs(app, &ctx);
}

/// The current team's card (also on the admin's All teams screen).
pub fn card(app: &mut App, ui: &mut Ui) {
    let Some(team) = app.team else { return };
    let name = app.team_name().unwrap_or_default();
    let manage = app.can_manage();
    let my_id = app.me.as_ref().and_then(|m| m.account_id);
    let owner = app
        .my_teams()
        .iter()
        .find(|t| t.id == team)
        .and_then(|t| t.owner.clone())
        .or_else(|| app.teams.iter().find(|t| t.id == team).and_then(|t| t.owner.clone()));
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.heading(&name);
            if let Some(r) = app.my_role() {
                widgets::badge(ui, r.label(), if r == TeamRole::Guest { theme::TEXT_WEAK } else { theme::ACCENT });
            }
        });
        let who = match &owner {
            Some(o) => format!("Owned by {o}"),
            None => "Made by the admin".to_owned(),
        };
        let limit = match app.choreography_limit() {
            Some(l) => format!(" · free plan: {l} choreographies"),
            None => " · unlimited choreographies".to_owned(),
        };
        ui.label(RichText::new(format!("{who}{limit}")).small().color(theme::TEXT_WEAK));
        if manage {
            ui.horizontal(|ui| {
                if ui.button("Rename…").clicked() {
                    app.team_ui.rename = Some(name.clone());
                }
                if ui.button(RichText::new("Delete team…").color(theme::ERROR)).clicked() {
                    app.team_ui.delete = Some(String::new());
                }
            });
        }

        widgets::section(ui, "People");
        let mut role_change = None;
        let mut remove = None;
        let mut leave = false;
        egui::Grid::new("team_members").striped(true).num_columns(3).spacing([18.0, 8.0]).show(ui, |ui| {
            for m in &app.team_ui.members {
                let me = Some(m.account_id) == my_id;
                ui.label(RichText::new(if me { format!("{} (you)", m.nickname) } else { m.nickname.clone() }).strong());
                if manage && m.role != TeamRole::Owner {
                    let mut r = m.role;
                    if role_picker(ui, &format!("role_{}", m.account_id), &mut r) {
                        role_change = Some((m.account_id, r));
                    }
                } else {
                    ui.label(RichText::new(m.role.label()).color(theme::TEXT_WEAK));
                }
                if m.role == TeamRole::Owner {
                    ui.label("");
                } else if me {
                    if ui.button("Leave team").clicked() {
                        leave = true;
                    }
                } else if manage {
                    if ui.button(RichText::new("Remove").color(theme::ERROR)).clicked() {
                        remove = Some(m.clone());
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
        if app.team_ui.members.is_empty() && app.team_ui.loaded_for == Some(team) {
            ui.label(RichText::new("Nobody yet.").color(theme::TEXT_WEAK));
        }
        if let Some((account, role)) = role_change {
            if let Some(m) = app.team_ui.members.iter_mut().find(|m| m.account_id == account) {
                m.role = role;
            }
            set_role(app, team, account, role);
        }
        if remove.is_some() {
            app.team_ui.remove = remove;
        }
        if leave {
            app.team_ui.leave = true;
        }

        if !manage {
            return;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Add someone by their nickname").strong());
        ui.horizontal_wrapped(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.team_ui.add_nickname).hint_text("Nickname").desired_width(180.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let mut role = app.team_ui.add_role;
            role_picker(ui, "add_role", &mut role);
            app.team_ui.add_role = role;
            let nick = app.team_ui.add_nickname.trim().to_owned();
            if (ui.add_enabled(!nick.is_empty(), egui::Button::new("Add")).clicked() || enter) && !nick.is_empty() {
                add_member(app, team, nick, role);
            }
        });
        ui.label(
            RichText::new("They need an account first. Co-owners edit the choreographies with you; guests only view them.")
                .small()
                .color(theme::TEXT_WEAK),
        );

        widgets::section(ui, "Share links");
        ui.label(
            RichText::new("Anyone with a link joins after logging in or signing up. Remove a link to stop it working.")
                .small()
                .color(theme::TEXT_WEAK),
        );
        let mut revoke_token = None;
        let mut copied = false;
        for i in &app.team_ui.invites {
            let url = link(i);
            ui.horizontal(|ui| {
                widgets::badge(ui, i.role.label(), if i.role == TeamRole::Guest { theme::TEXT_WEAK } else { theme::ACCENT });
                ui.label(RichText::new(&url).monospace().small());
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(url.clone());
                    copied = true;
                }
                if ui.small_button(RichText::new("Remove").color(theme::ERROR)).clicked() {
                    revoke_token = Some(i.token.clone());
                }
            });
        }
        if copied {
            app.toast("Link copied");
        }
        if let Some(t) = revoke_token {
            revoke(app, t);
        }
        ui.horizontal(|ui| {
            let mut role = app.team_ui.link_role;
            role_picker(ui, "link_role", &mut role);
            app.team_ui.link_role = role;
            if ui.button("Create link").clicked() {
                create_invite(app, team, role);
            }
        });
    });
}

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    let Some(team) = app.team else { return };
    if let Some(name) = &mut app.team_ui.rename {
        let valid = !name.trim().is_empty();
        let c = widgets::dialog(ctx, "rename_team", "Rename team", "Rename", false, valid, |ui| {
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        });
        let name = name.trim().to_owned();
        match c {
            Some(Choice::Confirm) => {
                app.team_ui.rename = None;
                rename(app, team, name);
            }
            Some(Choice::Cancel) => app.team_ui.rename = None,
            None => {}
        }
    }
    let team_name = app.team_name().unwrap_or_default();
    if let Some(typed) = &mut app.team_ui.delete {
        let name = team_name;
        let ok = typed.trim() == name.as_str();
        let c = widgets::dialog(ctx, "delete_team", "Delete team?", "Delete team", true, ok, |ui| {
            ui.label(format!(
                "This deletes “{name}” with all its choreographies and music. People in it keep their accounts. This cannot be undone."
            ));
            ui.add_space(6.0);
            ui.label(format!("Type the team name ({name}) to confirm:"));
            ui.add(egui::TextEdit::singleline(typed).desired_width(f32::INFINITY));
        });
        match c {
            Some(Choice::Confirm) => {
                app.team_ui.delete = None;
                delete_team(app, team);
            }
            Some(Choice::Cancel) => app.team_ui.delete = None,
            None => {}
        }
    }
    if let Some(m) = app.team_ui.remove.clone() {
        match widgets::dialog(ctx, "remove_member", "Remove from the team?", "Remove", true, true, |ui| {
            ui.label(format!("{} loses access to this team's choreographies. Their account stays.", m.nickname));
        }) {
            Some(Choice::Confirm) => {
                app.team_ui.remove = None;
                remove_member(app, team, m.account_id, false);
            }
            Some(Choice::Cancel) => app.team_ui.remove = None,
            None => {}
        }
    }
    if app.team_ui.leave {
        let name = app.team_name().unwrap_or_default();
        match widgets::dialog(ctx, "leave_team", "Leave the team?", "Leave", true, true, |ui| {
            ui.label(format!("You lose access to “{name}” until someone adds you again."));
        }) {
            Some(Choice::Confirm) => {
                app.team_ui.leave = false;
                if let Some(me) = app.me.as_ref().and_then(|m| m.account_id) {
                    remove_member(app, team, me, true);
                }
            }
            Some(Choice::Cancel) => app.team_ui.leave = false,
            None => {}
        }
    }
}
