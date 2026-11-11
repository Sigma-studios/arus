//! "Eightcount was updated": every open page (and the apps, which show this page)
//! asks the server which client it hands out, every minute and when the window
//! gets focus again. Once that differs from the version seen at start, a bar
//! offers to reload.

use std::time::Duration;

use arrus_api::Version;
use egui::{Frame, Margin, RichText, Ui};

use super::{App, net};
use crate::{theme, web};

/// Seconds between checks.
const EVERY: f64 = 60.0;
/// "Later" hides the bar this long.
const SNOOZE: f64 = 30.0 * 60.0;

#[derive(Default)]
pub struct UpdateWatch {
    /// The version this page was loaded with (the first answer).
    loaded: Option<String>,
    /// The server now hands out another one.
    newer: bool,
    checking: bool,
    next_check: f64,
    snoozed_until: f64,
    was_focused: bool,
}

/// Check when due; call every frame.
pub fn tick(app: &mut App, ctx: &egui::Context) {
    let (now, focused) = ctx.input(|i| (i.time, i.focused));
    let w = &mut app.update;
    // Coming back to the window (or the app on a phone) checks at once.
    let refocused = focused && !w.was_focused;
    w.was_focused = focused;
    if !w.checking && !w.newer && (now >= w.next_check || refocused) {
        w.checking = true;
        w.next_check = now + EVERY;
        app.net.spawn(async {
            let r = net::get::<Version>("/api/version").await;
            move |app: &mut App| {
                let w = &mut app.update;
                w.checking = false;
                // Offline or mid-deploy: try again at the next check.
                let Ok(v) = r else { return };
                match &w.loaded {
                    None => w.loaded = Some(v.client),
                    Some(l) if *l != v.client => w.newer = true,
                    Some(_) => {}
                }
            }
        });
    }
    if !w.newer {
        ctx.request_repaint_after(Duration::from_secs_f64(EVERY));
    }
}

/// The bar under the top bar, once there is a new version.
pub fn bar(app: &mut App, ui: &mut Ui) {
    let now = ui.input(|i| i.time);
    let w = &mut app.update;
    if !w.newer || now < w.snoozed_until {
        return;
    }
    let fill = theme::accent();
    let text = theme::on_color(fill);
    let compact = app.compact;
    egui::Panel::top("update_bar")
        .frame(Frame::new().fill(fill).inner_margin(Margin::symmetric(14, 8)))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if compact {
                        "Eightcount was updated."
                    } else {
                        "Eightcount was updated. Reload to use the new version."
                    })
                    .color(text)
                    .strong(),
                );
                ui.add_space(8.0);
                if ui.add(egui::Button::new(RichText::new("Reload now").strong()).fill(theme::surface())).clicked() {
                    web::reload();
                }
                if ui.add(egui::Button::new(RichText::new("Later").color(text)).frame(false)).clicked() {
                    w.snoozed_until = now + SNOOZE;
                }
            });
        });
}
