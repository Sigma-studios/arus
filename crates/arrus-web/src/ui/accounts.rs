//! Admin: every account. Search, premium and admin rights, email, passwords,
//! and the teams and choreographies of each.

use arrus_api::{AccountId, AccountInfo, CreateAccount, MIN_PASSWORD_LEN, TeamRole, UpdateAccount};
use egui::{Frame, Margin, RichText, ScrollArea, Ui};

use super::widgets::{self, Choice};
use super::{App, Screen, editor, net};
use crate::{theme, web};

#[derive(Default)]
pub struct Accounts {
    pub list: Vec<AccountInfo>,
    loading: bool,
    search: String,
    create: Option<CreateForm>,
    /// Editing nickname and email.
    edit: Option<(AccountId, String, String)>,
    /// Setting a password: the account, its nickname, what is typed.
    set_password: Option<(AccountId, String, String)>,
    regenerate: Option<(AccountId, String)>,
    delete: Option<AccountInfo>,
}

#[derive(Default)]
struct CreateForm {
    nickname: String,
    email: String,
    password: String,
    busy: bool,
}

pub fn load(app: &mut App) {
    app.accounts.loading = true;
    app.net.spawn(async move {
        let r = net::get::<Vec<AccountInfo>>("/api/accounts").await;
        move |app: &mut App| {
            app.accounts.loading = false;
            match r {
                Ok(list) => app.accounts.list = list,
                Err(e) => app.fail(e),
            }
        }
    });
}

fn update(app: &mut App, id: AccountId, body: UpdateAccount) {
    let new_password = body.password.is_some() || body.regenerate_password;
    let rights = body.is_admin.is_some() || body.is_premium.is_some();
    app.net.spawn(async move {
        let r = net::send::<_, AccountInfo>("PATCH", &format!("/api/accounts/{id}"), &body).await;
        move |app: &mut App| match r {
            Ok(a) => {
                if new_password && let Some(p) = &a.password {
                    app.toast(format!("New password for {}: {p}", a.nickname));
                }
                if let Some(x) = app.accounts.list.iter_mut().find(|x| x.id == a.id) {
                    *x = a;
                }
                if rights {
                    // Limits of their teams change with premium.
                    app.load_teams();
                    app.reload_me();
                }
            }
            Err(net::ApiError::Http { code: Some(c), .. }) if c == "username_taken" => {
                app.toast_error("That nickname is already taken.");
                load(app);
            }
            Err(net::ApiError::Http { code: Some(c), .. }) if c == "email_taken" => {
                app.toast_error("Another account already uses that email.");
                load(app);
            }
            Err(e) => {
                app.fail(e);
                load(app);
            }
        }
    });
}

fn create(app: &mut App, body: CreateAccount) {
    app.net.spawn(async move {
        let r = net::send::<_, AccountInfo>("POST", "/api/accounts", &body).await;
        move |app: &mut App| {
            if let Some(f) = &mut app.accounts.create {
                f.busy = false;
            }
            match r {
                Ok(a) => {
                    app.accounts.create = None;
                    app.toast(format!("Created {}. Password: {}", a.nickname, a.password.clone().unwrap_or_default()));
                    app.accounts.search.clear();
                    app.accounts.list.push(a);
                    app.accounts.list.sort_by_key(|a| a.nickname.to_lowercase());
                }
                Err(net::ApiError::Http { code: Some(c), .. }) if c == "username_taken" => {
                    app.toast_error("That nickname is already taken.")
                }
                Err(net::ApiError::Http { code: Some(c), .. }) if c == "email_taken" => {
                    app.toast_error("Another account already uses that email.")
                }
                Err(e) => app.fail(e),
            }
        }
    });
}

fn delete(app: &mut App, id: AccountId) {
    app.net.spawn(async move {
        let r = net::send_empty("DELETE", &format!("/api/accounts/{id}")).await;
        move |app: &mut App| match r {
            Ok(()) => {
                app.accounts.list.retain(|a| a.id != id);
                app.toast("Account deleted");
                app.load_teams();
                app.refresh_storage();
            }
            Err(e) => app.fail(e),
        }
    });
}

fn matches(a: &AccountInfo, q: &str) -> bool {
    q.is_empty()
        || a.nickname.to_lowercase().contains(q)
        || a.email.as_deref().is_some_and(|e| e.to_lowercase().contains(q))
        || a.teams.iter().any(|t| t.name.to_lowercase().contains(q))
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let compact = app.compact;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Accounts");
                ui.label(RichText::new(app.accounts.list.len().to_string()).size(18.0).color(theme::TEXT_WEAK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, "+ Create account").clicked() {
                        app.accounts.create = Some(CreateForm::default());
                    }
                    if ui.button("Refresh").clicked() {
                        load(app);
                    }
                    if app.accounts.loading {
                        ui.spinner();
                    }
                });
            });
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut app.accounts.search)
                    .hint_text("Search nickname, email or team…")
                    .desired_width(320.0),
            );
            ui.add_space(8.0);
            let q = app.accounts.search.trim().to_lowercase();
            let shown: Vec<AccountInfo> = app.accounts.list.iter().filter(|a| matches(a, &q)).cloned().collect();
            if shown.is_empty() && !app.accounts.loading {
                ui.label(RichText::new(if q.is_empty() { "No accounts yet." } else { "No account matches." }).color(theme::TEXT_WEAK));
            }
            let my_id = app.me.as_ref().and_then(|m| m.account_id);
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(900.0);
                for a in &shown {
                    account_card(app, ui, a, my_id == Some(a.id));
                    ui.add_space(8.0);
                }
            });
        });
    dialogs(app, &ctx);
}

fn account_card(app: &mut App, ui: &mut Ui, a: &AccountInfo, is_me: bool) {
    let mut open_team = None;
    let mut open_choreo = None;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&a.nickname).strong().size(16.0));
            if a.is_admin {
                widgets::badge(ui, "ADMIN", theme::SELECT);
            }
            if a.is_premium {
                widgets::badge(ui, "PREMIUM", theme::ACCENT);
            }
            match &a.email {
                Some(e) => ui.label(RichText::new(e).color(theme::TEXT_WEAK)),
                None => ui.label(RichText::new("no email").color(theme::TEXT_FAINT)),
            };
            ui.label(RichText::new(format!("since {}", web::local_datetime(a.created_at))).small().color(theme::TEXT_FAINT));
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Password").small().color(theme::TEXT_FAINT));
            match &a.password {
                Some(p) => {
                    ui.label(RichText::new(p).monospace());
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(p.clone());
                        app.toast("Copied");
                    }
                }
                None => {
                    ui.label(RichText::new("chosen by them (hidden)").color(theme::TEXT_WEAK))
                        .on_hover_text("People's own passwords are stored encrypted. Set a new one to see it.");
                }
            }
            if ui.small_button("Set…").on_hover_text("Choose a new password (you'll see it; they are logged out)").clicked() {
                app.accounts.set_password = Some((a.id, a.nickname.clone(), String::new()));
            }
            if ui.small_button("Generate").on_hover_text("A new easy password like tiger-maple-42").clicked() {
                app.accounts.regenerate = Some((a.id, a.nickname.clone()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut premium = a.is_premium;
            if ui.checkbox(&mut premium, "Premium").on_hover_text("Unlimited teams and choreographies").changed() {
                update(app, a.id, UpdateAccount { is_premium: Some(premium), ..Default::default() });
            }
            let mut admin = a.is_admin;
            let r = ui.add_enabled(!is_me, egui::Checkbox::new(&mut admin, "Admin"));
            if r.on_hover_text("Sees and manages everything, like you").on_disabled_hover_text("You cannot remove your own admin rights").changed() {
                update(app, a.id, UpdateAccount { is_admin: Some(admin), ..Default::default() });
            }
            ui.separator();
            if ui.button("Edit…").clicked() {
                app.accounts.edit = Some((a.id, a.nickname.clone(), a.email.clone().unwrap_or_default()));
            }
            if ui.add_enabled(!is_me, egui::Button::new(RichText::new("Delete…").color(theme::ERROR))).clicked() {
                app.accounts.delete = Some(a.clone());
            }
        });
        let n: usize = a.teams.iter().map(|t| t.choreographies.len()).sum();
        let title = format!(
            "{} team{} · {n} choreograph{}",
            a.teams.len(),
            if a.teams.len() == 1 { "" } else { "s" },
            if n == 1 { "y" } else { "ies" }
        );
        if a.teams.is_empty() {
            ui.label(RichText::new("In no team").small().color(theme::TEXT_FAINT));
            return;
        }
        egui::CollapsingHeader::new(RichText::new(title).small())
            .id_salt(("account_teams", a.id))
            .show(ui, |ui| {
                for t in &a.teams {
                    ui.horizontal_wrapped(|ui| {
                        if ui.link(RichText::new(&t.name).strong()).on_hover_text("Open this team's library").clicked() {
                            open_team = Some(t.team_id);
                        }
                        let color = if t.role == TeamRole::Guest { theme::TEXT_WEAK } else { theme::ACCENT };
                        widgets::badge(ui, t.role.label(), color);
                    });
                    ui.indent(("account_team", a.id, t.team_id), |ui| {
                        if t.choreographies.is_empty() {
                            ui.label(RichText::new("no choreographies").small().color(theme::TEXT_FAINT));
                        }
                        ui.horizontal_wrapped(|ui| {
                            for c in &t.choreographies {
                                if ui.link(&c.name).on_hover_text("Open").clicked() {
                                    open_choreo = Some((t.team_id, c.id));
                                }
                            }
                        });
                    });
                }
            });
    });
    if let Some(team) = open_team {
        app.set_team(Some(team));
        app.go(Screen::Library);
    }
    if let Some((team, id)) = open_choreo {
        app.set_team(Some(team));
        editor::open(app, id);
    }
}

fn dialogs(app: &mut App, ctx: &egui::Context) {
    if let Some(f) = &mut app.accounts.create {
        let pw_ok = f.password.is_empty() || f.password.chars().count() >= MIN_PASSWORD_LEN;
        let valid = !f.nickname.trim().is_empty() && pw_ok && !f.busy;
        let c = widgets::dialog(ctx, "create_account", "Create account", "Create", false, valid, |ui| {
            egui::Grid::new("create_account_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Nickname");
                ui.add(egui::TextEdit::singleline(&mut f.nickname).desired_width(220.0));
                ui.end_row();
                ui.label("Email");
                ui.add(egui::TextEdit::singleline(&mut f.email).hint_text("optional").desired_width(220.0));
                ui.end_row();
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut f.password).hint_text("empty: generate one").desired_width(220.0));
                ui.end_row();
            });
            if !pw_ok {
                ui.label(RichText::new(format!("At least {MIN_PASSWORD_LEN} characters.")).small().color(theme::WARN));
            }
        });
        match c {
            Some(Choice::Confirm) => {
                f.busy = true;
                let body = CreateAccount {
                    nickname: f.nickname.trim().to_owned(),
                    email: Some(f.email.trim().to_owned()).filter(|e| !e.is_empty()),
                    password: Some(f.password.clone()).filter(|p| !p.is_empty()),
                };
                create(app, body);
            }
            Some(Choice::Cancel) => app.accounts.create = None,
            None => {}
        }
    }

    if let Some((id, nickname, email)) = &mut app.accounts.edit {
        let id = *id;
        let valid = !nickname.trim().is_empty();
        let c = widgets::dialog(ctx, "edit_account", "Edit account", "Save", false, valid, |ui| {
            egui::Grid::new("edit_account_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Nickname");
                ui.add(egui::TextEdit::singleline(nickname).desired_width(220.0));
                ui.end_row();
                ui.label("Email");
                ui.add(egui::TextEdit::singleline(email).hint_text("none").desired_width(220.0));
                ui.end_row();
            });
            ui.label(RichText::new("They log in with either.").small().color(theme::TEXT_WEAK));
        });
        let body = UpdateAccount {
            nickname: Some(nickname.trim().to_owned()),
            email: Some(email.trim().to_owned()),
            ..Default::default()
        };
        match c {
            Some(Choice::Confirm) => {
                app.accounts.edit = None;
                update(app, id, body);
            }
            Some(Choice::Cancel) => app.accounts.edit = None,
            None => {}
        }
    }

    if let Some((id, nickname, pw)) = &mut app.accounts.set_password {
        let id = *id;
        let valid = pw.trim().chars().count() >= MIN_PASSWORD_LEN;
        let n = nickname.clone();
        let c = widgets::dialog(ctx, "set_password", &format!("New password for {n}"), "Set password", false, valid, |ui| {
            ui.add(egui::TextEdit::singleline(pw).desired_width(f32::INFINITY));
            ui.label(
                RichText::new(format!(
                    "At least {MIN_PASSWORD_LEN} characters. You keep seeing it until they change it; they are logged out everywhere."
                ))
                .small()
                .color(theme::TEXT_WEAK),
            );
        });
        let body = UpdateAccount { password: Some(pw.trim().to_owned()), ..Default::default() };
        match c {
            Some(Choice::Confirm) => {
                app.accounts.set_password = None;
                update(app, id, body);
            }
            Some(Choice::Cancel) => app.accounts.set_password = None,
            None => {}
        }
    }

    if let Some((id, nickname)) = app.accounts.regenerate.clone() {
        match widgets::dialog(ctx, "regen_pw", "Generate a new password?", "Generate", false, true, |ui| {
            ui.label(format!("{nickname} will be logged out everywhere and need the new password."));
        }) {
            Some(Choice::Confirm) => {
                app.accounts.regenerate = None;
                update(app, id, UpdateAccount { regenerate_password: true, ..Default::default() });
            }
            Some(Choice::Cancel) => app.accounts.regenerate = None,
            None => {}
        }
    }

    if let Some(a) = app.accounts.delete.clone() {
        let owned: Vec<&str> = a.teams.iter().filter(|t| t.role == TeamRole::Owner).map(|t| t.name.as_str()).collect();
        match widgets::dialog(ctx, "delete_account", "Delete account?", "Delete", true, true, |ui| {
            ui.label(format!("{} can no longer log in.", a.nickname));
            if !owned.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "The teams they own are deleted too, with their choreographies and music: {}.",
                        owned.join(", ")
                    ))
                    .color(theme::ERROR),
                );
            }
            ui.label(RichText::new("This cannot be undone.").small().color(theme::TEXT_WEAK));
        }) {
            Some(Choice::Confirm) => {
                app.accounts.delete = None;
                delete(app, a.id);
            }
            Some(Choice::Cancel) => app.accounts.delete = None,
            None => {}
        }
    }
}
