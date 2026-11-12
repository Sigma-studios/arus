//! The plans side by side: what the free plan gives, what premium adds.

use arrus_api::{
    FREE_CHOREOGRAPHIES, FREE_TEAMS, PREMIUM_CHOREOGRAPHIES, PREMIUM_MUSIC_BYTES, PREMIUM_TEAMS, PlanLimits,
};
use egui::{Color32, Frame, Margin, RichText, ScrollArea, Stroke, Ui};

use super::{App, widgets};
use crate::{theme, util};

/// Where people can ask about premium.
pub const CONTACT: &str = "contact@eightcount.de";

enum Cell {
    Yes(String),
    No(String),
}

/// One row: the feature and what each plan offers.
fn features() -> Vec<(&'static str, Cell, Cell)> {
    let plural = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    vec![
        (
            "Teams you create",
            Cell::Yes(plural(FREE_TEAMS, "team", "teams")),
            Cell::Yes(format!("Up to {PREMIUM_TEAMS} teams")),
        ),
        (
            "Choreographies per team",
            Cell::Yes(plural(FREE_CHOREOGRAPHIES, "choreography", "choreographies")),
            Cell::Yes(format!("Up to {PREMIUM_CHOREOGRAPHIES} choreographies")),
        ),
        (
            "Music",
            Cell::No("No music".into()),
            Cell::Yes(format!("{} for your tracks", util::fmt_bytes(PREMIUM_MUSIC_BYTES))),
        ),
        ("Edit live together", Cell::Yes("Included".into()), Cell::Yes("Included".into())),
        ("Invite people (co-owners, guests)", Cell::Yes("Included".into()), Cell::Yes("Included".into())),
        ("Import from ArrangeUs, export .arrus and PDF", Cell::Yes("Included".into()), Cell::Yes("Included".into())),
        ("Apps for Windows, macOS, Linux, Android, iPhone", Cell::Yes("Included".into()), Cell::Yes("Included".into())),
    ]
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let compact = app.compact;
    let me = app.me.clone();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Eightcount Premium").size(26.0).strong().color(theme::accent()));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("More teams, more choreographies, and your music in every formation.")
                            .color(theme::TEXT_WEAK),
                    );
                });
                ui.add_space(18.0);
                let premium = me.as_ref().is_some_and(|m| m.premium);
                let gap = 16.0;
                let max_w = 860.0f32.min(ui.available_width());
                let side = (ui.available_width() - max_w) / 2.0;
                if compact || max_w < 560.0 {
                    plan_card(ui, false, !premium);
                    ui.add_space(gap);
                    plan_card(ui, true, premium);
                } else {
                    let w = (max_w - gap) / 2.0;
                    ui.horizontal_top(|ui| {
                        ui.add_space(side);
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            plan_card(ui, false, !premium);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            plan_card(ui, true, premium);
                        });
                    });
                }
                if let Some(me) = &me {
                    ui.add_space(gap);
                    ui.horizontal_top(|ui| {
                        if !compact {
                            ui.add_space(side.max(0.0));
                        }
                        ui.vertical(|ui| {
                            ui.set_width(if compact { ui.available_width() } else { max_w });
                            usage_card(ui, me);
                        });
                    });
                }
            });
        });
}

fn plan_card(ui: &mut Ui, premium: bool, current: bool) {
    let accent = theme::accent();
    let stroke = if premium { Stroke::new(2.0, accent) } else { Stroke::new(1.0, theme::border()) };
    widgets::card().stroke(stroke).inner_margin(Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let name = if premium { "Premium" } else { "Free" };
            ui.label(RichText::new(name).size(22.0).strong().color(if premium { accent } else { theme::TEXT }));
            if current {
                widgets::badge(ui, "YOUR PLAN", if premium { accent } else { theme::TEXT_WEAK });
            }
        });
        ui.add_space(4.0);
        if premium {
            ui.label(RichText::new("Coming soon").size(20.0).strong().color(theme::TEXT));
            ui.label(RichText::new("Price to be announced").small().color(theme::TEXT_WEAK));
        } else {
            ui.label(RichText::new("0 €").size(20.0).strong().color(theme::TEXT));
            ui.label(RichText::new("Free, forever").small().color(theme::TEXT_WEAK));
        }
        ui.add_space(12.0);
        for (feature, free, paid) in features() {
            let cell = if premium { paid } else { free };
            let (mark, color, text, has) = match cell {
                Cell::Yes(t) => ("✔", theme::OK, t, true),
                Cell::No(t) => ("✖", theme::TEXT_FAINT, t, false),
            };
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(mark).color(color));
                ui.vertical(|ui| {
                    ui.label(RichText::new(feature).color(theme::TEXT_WEAK).small());
                    ui.label(if has { RichText::new(text) } else { RichText::new(text).color(theme::TEXT_FAINT) });
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(10.0);
        if premium {
            let fill = accent;
            ui.add_enabled(
                false,
                egui::Button::new(RichText::new("Coming soon").strong().color(theme::on_color(fill)))
                    .fill(fill)
                    .min_size(egui::vec2(ui.available_width(), 36.0)),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("Want premium already? Write to {CONTACT}."))
                    .small()
                    .color(theme::TEXT_WEAK),
            );
        } else {
            ui.add_enabled(
                false,
                egui::Button::new(if current { "Your plan" } else { "Free plan" })
                    .fill(Color32::TRANSPARENT)
                    .min_size(egui::vec2(ui.available_width(), 36.0)),
            );
        }
    });
}

/// What this account uses of its plan.
fn usage_card(ui: &mut Ui, me: &arrus_api::Me) {
    let limits: PlanLimits = me.limits;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section(ui, "What you use");
        let row = |ui: &mut Ui, label: &str, used: f64, limit: Option<f64>, text: String| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(theme::TEXT_WEAK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(text);
                    if let Some(l) = limit.filter(|l| *l > 0.0) {
                        let frac = (used / l).min(1.0) as f32;
                        let col = if frac >= 1.0 { theme::WARN } else { theme::accent() };
                        widgets::meter(ui, egui::vec2(120.0, 7.0), frac, col);
                    }
                });
            });
        };
        let of = |n: Option<u32>| n.map(|l| format!(" of {l}")).unwrap_or_default();
        row(ui, "Teams you own", me.teams_owned as f64, limits.teams.map(f64::from), format!("{}{}", me.teams_owned, of(limits.teams)));
        let most = me.teams.iter().filter(|t| t.choreography_limit.is_some()).map(|t| t.choreography_count).max();
        if let (Some(n), Some(l)) = (most, limits.choreographies_per_team) {
            row(ui, "Choreographies (fullest team)", n as f64, Some(l as f64), format!("{n} of {l}"));
        }
        match limits.music_bytes {
            Some(0) => row(ui, "Music", 0.0, None, "Premium only".into()),
            Some(l) => row(
                ui,
                "Music",
                me.music_used as f64,
                Some(l as f64),
                format!("{} of {}", util::fmt_bytes(me.music_used), util::fmt_bytes(l)),
            ),
            None => row(ui, "Music", 0.0, None, format!("{} (no limit)", util::fmt_bytes(me.music_used))),
        }
    });
}
