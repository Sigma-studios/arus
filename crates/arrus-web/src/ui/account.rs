//! Account settings: the logged-in account changes its email or password
//! (the nickname is in [`super::settings`]: once a month), or deletes itself.

use arrus_api::{DeleteMe, MIN_PASSWORD_LEN, Me, TeamRole, UpdateMe};
use egui::RichText;

use super::widgets::{self, Choice};
use super::{App, net};
use crate::theme;

#[derive(Default)]
pub struct AccountUi {
    open: bool,
    busy: bool,
    email: String,
    new_password: String,
    new_password2: String,
    current: String,
    error: Option<String>,
    /// "Delete my account": the password typed, and whether it is being deleted.
    delete: Option<DeleteForm>,
}

#[derive(Default)]
struct DeleteForm {
    password: String,
    confirm: bool,
    busy: bool,
    error: Option<String>,
}

pub fn open(app: &mut App) {
    let Some(me) = &app.me else { return };
    app.account = AccountUi {
        open: true,
        email: me.email.clone().unwrap_or_default(),
        ..Default::default()
    };
}

fn save(app: &mut App, body: UpdateMe) {
    app.account.busy = true;
    app.account.error = None;
    let password_changed = body.new_password.is_some();
    let email_changed = body.email.is_some();
    app.net.spawn(async move {
        let r = net::send::<_, Me>("PATCH", "/api/me", &body).await;
        move |app: &mut App| {
            app.account.busy = false;
            match r {
                Ok(me) => {
                    let confirm = (email_changed && me.mail && !me.email_verified).then(|| me.email.clone()).flatten();
                    app.me = Some(me);
                    app.account.open = false;
                    app.toast(match (confirm, password_changed) {
                        (Some(to), _) => format!("Saved. Open the link we sent to {to} to confirm it."),
                        (None, true) => "Saved. Other devices need the new password.".into(),
                        (None, false) => "Saved".into(),
                    });
                }
                Err(net::ApiError::Unauthorized) => crate::web::navigate("/login"),
                Err(e) => app.account.error = Some(e.to_string()),
            }
        }
    });
}

pub fn dialog(app: &mut App, ctx: &egui::Context) {
    delete_dialog(app, ctx);
    if !app.account.open {
        return;
    }
    let Some(me) = app.me.clone() else { return };
    let a = &mut app.account;
    let pw_mismatch = !a.new_password.is_empty() && a.new_password != a.new_password2;
    let pw_short = !a.new_password.is_empty() && a.new_password.chars().count() < MIN_PASSWORD_LEN;
    let changed = a.email.trim() != me.email.as_deref().unwrap_or("")
        || !a.new_password.is_empty();
    let valid = changed && !a.current.is_empty() && !pw_mismatch && !pw_short && !a.busy;
    let mut send_link = false;
    let mut open_delete = false;
    let c = widgets::dialog(ctx, "account_settings", "Account settings", "Save", false, valid, |ui| {
        egui::Grid::new("account_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("Email");
            ui.add(egui::TextEdit::singleline(&mut a.email).hint_text("you@example.com").desired_width(200.0));
            ui.end_row();
            // Whether the saved address is confirmed (not shown while typing a new one).
            if me.mail && me.email.is_some() && a.email.trim() == me.email.as_deref().unwrap_or("") {
                ui.label("");
                ui.horizontal(|ui| {
                    if me.email_verified {
                        ui.label(RichText::new("✓ Confirmed").small().color(theme::OK));
                    } else {
                        ui.label(RichText::new("Not confirmed yet").small().color(theme::WARN));
                        if ui.small_button("Send the link again").clicked() {
                            send_link = true;
                        }
                    }
                });
                ui.end_row();
            }
            ui.label("New password");
            ui.add(egui::TextEdit::singleline(&mut a.new_password).password(true).hint_text("unchanged").desired_width(200.0));
            ui.end_row();
            ui.label("Again");
            ui.add(egui::TextEdit::singleline(&mut a.new_password2).password(true).desired_width(200.0));
            ui.end_row();
        });
        if pw_short {
            ui.label(RichText::new(format!("At least {MIN_PASSWORD_LEN} characters.")).small().color(theme::WARN));
        } else if pw_mismatch {
            ui.label(RichText::new("The two passwords are different.").small().color(theme::WARN));
        }
        ui.add_space(6.0);
        ui.separator();
        ui.label("Current password (to confirm)");
        ui.add(egui::TextEdit::singleline(&mut a.current).password(true).desired_width(f32::INFINITY));
        if let Some(e) = &a.error {
            ui.label(RichText::new(e).color(theme::ERROR));
        }
        if me.mail && a.email.trim() != me.email.as_deref().unwrap_or("") && !a.email.trim().is_empty() {
            ui.label(
                RichText::new("We'll send a link to the new address to confirm it.").small().color(theme::TEXT_FAINT),
            );
        }
        ui.label(
            RichText::new("A password you choose is stored encrypted: nobody, the admin included, can read it.")
                .small()
                .color(theme::TEXT_FAINT),
        );
        ui.add_space(10.0);
        ui.separator();
        if ui.add(egui::Button::new(RichText::new("Delete my account…").color(theme::ERROR)).frame(false)).clicked() {
            open_delete = true;
        }
    });
    if send_link {
        super::confirm::send(app);
    }
    if open_delete {
        app.account.open = false;
        app.account.delete = Some(DeleteForm::default());
        return;
    }
    match c {
        Some(Choice::Confirm) => {
            let a = &app.account;
            let body = UpdateMe {
                current_password: a.current.clone(),
                nickname: None,
                email: (a.email.trim() != me.email.as_deref().unwrap_or("")).then(|| a.email.trim().to_owned()),
                new_password: (!a.new_password.is_empty()).then(|| a.new_password.clone()),
            };
            save(app, body);
        }
        Some(Choice::Cancel) => app.account.open = false,
        None => {}
    }
}

fn delete_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(me) = app.me.clone() else { return };
    let Some(f) = &mut app.account.delete else { return };
    let owned: Vec<&str> = me.teams.iter().filter(|t| t.role == TeamRole::Owner).map(|t| t.name.as_str()).collect();
    let valid = f.confirm && !f.password.is_empty() && !f.busy;
    let mut enter = false;
    let c = widgets::dialog(ctx, "delete_account", "Delete your account?", "Delete my account", true, valid, |ui| {
        ui.label(format!("This deletes the account {} for good. It can't be undone.", me.username));
        ui.add_space(6.0);
        if owned.is_empty() {
            ui.label(RichText::new("You own no teams. Teams others own stay as they are.").small().color(theme::TEXT_WEAK));
        } else {
            ui.label(RichText::new("These teams you own are deleted too, with all their choreographies and music, for everyone in them:").small());
            for t in &owned {
                ui.label(RichText::new(format!("• {t}")).strong());
            }
            ui.label(
                RichText::new("To keep choreographies, export them first (.arrus or PDF, in the library).")
                    .small()
                    .color(theme::TEXT_WEAK),
            );
        }
        ui.add_space(8.0);
        ui.label("Your password");
        let r = ui.add(egui::TextEdit::singleline(&mut f.password).password(true).desired_width(f32::INFINITY));
        enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.checkbox(&mut f.confirm, "I understand that everything listed is deleted for good");
        if let Some(e) = &f.error {
            ui.label(RichText::new(e).color(theme::ERROR));
        }
        if f.busy {
            ui.spinner();
        }
    });
    let go = matches!(c, Some(Choice::Confirm)) || (enter && valid);
    if matches!(c, Some(Choice::Cancel)) {
        app.account.delete = None;
        return;
    }
    if !go {
        return;
    }
    f.busy = true;
    f.error = None;
    let body = DeleteMe { password: f.password.clone() };
    app.net.spawn(async move {
        let r = net::send_empty_json("DELETE", "/api/me", &body).await;
        move |app: &mut App| match r {
            Ok(()) => crate::web::navigate("/account-deleted"),
            Err(net::ApiError::Unauthorized) => crate::web::navigate("/login"),
            Err(e) => {
                if let Some(f) = &mut app.account.delete {
                    f.busy = false;
                    f.error = Some(match &e {
                        net::ApiError::Http { code: Some(c), .. } if c == "wrong_password" => "That password is wrong.".into(),
                        e => e.to_string(),
                    });
                }
            }
        }
    });
}
