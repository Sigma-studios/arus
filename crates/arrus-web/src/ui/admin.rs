//! Admin: every team (and the current one's people), and where storage alerts go.

use arrus_api::{AlertSettings, UpdateAlertSettings};
use egui::{Frame, Margin, RichText, ScrollArea, Ui};

use super::widgets::{self, Choice};
use super::{App, net, team};
use crate::{theme, util};

#[derive(Default)]
pub struct Admin {
    new_team: String,
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

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(28, 18)))
        .show(ui, |ui| {
            ui.heading("All teams");
            ui.add_space(10.0);
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
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
                        team::card(app, ui);
                    });
                });
            });
        });
    team::dialogs(app, &ctx);
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
                let owner = t.owner.as_deref().map(|o| format!("{o}'s · ")).unwrap_or_default();
                let limit = t.choreography_limit.map(|l| format!(" of {l}")).unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "{owner}{} people · {}{limit} choreographies · {}",
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
                app.admin.new_team.clear();
                app.create_team(name);
            }
        });
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

fn dialogs(app: &mut App, ctx: &egui::Context) {
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
}
