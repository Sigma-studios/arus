//! "Please confirm your email": a bar for accounts whose address is not
//! confirmed yet (only when the server sends emails), with "Send again".
//! Coming back to the window (e.g. from the mail app) reloads the account,
//! so the bar goes away once the link was opened.

use egui::{Frame, Margin, RichText, Ui};

use super::{App, net};
use crate::{theme, web};

#[derive(Default)]
pub struct ConfirmUi {
    /// "Later": hidden until the page is loaded again.
    dismissed: bool,
    sending: bool,
    was_focused: bool,
}

fn needed(app: &App) -> bool {
    app.me.as_ref().is_some_and(|m| m.mail && m.email.is_some() && !m.email_verified)
}

/// Send the link (again).
pub fn send(app: &mut App) {
    app.confirm.sending = true;
    app.net.spawn(async {
        let r = net::send_empty("POST", "/api/me/confirm-email").await;
        move |app: &mut App| {
            app.confirm.sending = false;
            match r {
                Ok(()) => {
                    let to = app.me.as_ref().and_then(|m| m.email.clone()).unwrap_or_default();
                    app.toast(format!("Sent to {to}: open the link in it"));
                }
                Err(net::ApiError::Unauthorized) => web::navigate("/login"),
                Err(e) => app.toast_error(e.to_string()),
            }
        }
    });
}

pub fn bar(app: &mut App, ui: &mut Ui) {
    let focused = ui.input(|i| i.focused);
    let refocused = focused && !app.confirm.was_focused;
    app.confirm.was_focused = focused;
    if !needed(app) {
        return;
    }
    if refocused {
        app.reload_me();
    }
    if app.confirm.dismissed {
        return;
    }
    let email = app.me.as_ref().and_then(|m| m.email.clone()).unwrap_or_default();
    let compact = app.compact;
    let mut send_now = false;
    egui::Panel::top("confirm_bar")
        .frame(Frame::new().fill(theme::surface()).inner_margin(Margin::symmetric(14, 8)))
        .show_separator_line(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("✉").color(theme::WARN));
                if compact {
                    ui.label(RichText::new("Please confirm your email.").strong());
                } else {
                    ui.label(RichText::new(format!("Please confirm your email: we sent a link to {email}.")).strong());
                    ui.label(
                        RichText::new("It lets you get back into your account if you forget your password.")
                            .small()
                            .color(theme::TEXT_WEAK),
                    );
                }
                ui.add_space(6.0);
                if ui.add_enabled(!app.confirm.sending, egui::Button::new("Send it again")).clicked() {
                    send_now = true;
                }
                if ui.add(egui::Button::new(RichText::new("Later").color(theme::TEXT_WEAK)).frame(false)).clicked() {
                    app.confirm.dismissed = true;
                }
            });
        });
    if send_now {
        send(app);
    }
}
