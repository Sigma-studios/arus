//! Small shared widgets.

use egui::{Color32, CornerRadius, Id, Margin, Response, RichText, Stroke, Ui, Vec2};

use crate::theme;

/// Phones: hide the browser bars. Real fullscreen where the browser allows
/// it; otherwise (iPhone) how to open arus from the home screen instead.
/// Nothing when already running as a home-screen app.
pub fn fullscreen_button(ui: &mut Ui) {
    if crate::web::standalone() {
        return;
    }
    if crate::web::fullscreen_supported() {
        let on = crate::web::is_fullscreen();
        if ui.add(egui::Button::selectable(on, "Fullscreen")).clicked() {
            crate::web::toggle_fullscreen();
        }
    } else {
        ui.menu_button("Fullscreen", |ui| {
            ui.set_max_width(260.0);
            ui.label(RichText::new("Fullscreen on iPhone").strong());
            ui.label("Safari doesn't let websites hide its bars. Open arus as an app instead:");
            ui.label("1. Tap the Share button");
            ui.label("2. Choose “Add to Home Screen”");
            ui.label("3. Open arus from the new icon");
            ui.label(RichText::new("It then runs without the address bar. You log in once in the app.").small().color(theme::TEXT_WEAK));
        });
    }
}

/// Coloured initials of the other people with the choreography open (at
/// most `max`, then "+n"), for a right-to-left layout.
pub fn presence(ui: &mut Ui, others: &[&arrus_api::Viewer], max: usize) {
    if others.is_empty() {
        return;
    }
    let extra = others.len().saturating_sub(max);
    if extra > 0 {
        let names: Vec<&str> = others[max..].iter().map(|v| v.username.as_str()).collect();
        ui.label(RichText::new(format!("+{extra}")).color(theme::TEXT_WEAK)).on_hover_text(names.join(", "));
    }
    ui.spacing_mut().item_spacing.x = 3.0;
    for v in others.iter().take(max).rev() {
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
        let color = theme::person_color(&v.username);
        let p = ui.painter();
        p.circle_filled(r.center(), 12.5, color);
        p.circle_stroke(r.center(), 12.5, Stroke::new(1.5, theme::PANEL));
        let initial: String = v.username.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        p.text(r.center(), egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(13.0), Color32::WHITE);
        let doing = if v.can_write { "editing" } else { "viewing" };
        let at = v.formation.map(|f| format!(" · formation {}", f + 1)).unwrap_or_default();
        let holding = match v.selected.len() {
            0 => String::new(),
            n if v.dragging => format!(" · moving {n} dancer{}", if n == 1 { "" } else { "s" }),
            n => format!(" · {n} dancer{} selected", if n == 1 { "" } else { "s" }),
        };
        resp.on_hover_text(format!("{} · {doing}{at}{holding}", v.username));
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(theme::primary_fill()))
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(theme::ERROR.gamma_multiply(0.85)))
}

/// Small uppercase section title.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).small().strong().color(theme::TEXT_FAINT));
    ui.add_space(2.0);
}

/// A rounded pill with text.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text).small().strong().color(color)))
        .response
}

/// A card-like frame for content blocks.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
}

pub enum Choice {
    Confirm,
    Cancel,
}

/// A modal dialog with a title and custom body; returns the user's choice
/// when one is made (Escape / backdrop click = cancel).
pub fn dialog(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    confirm: &str,
    danger: bool,
    confirm_enabled: bool,
    body: impl FnOnce(&mut Ui),
) -> Option<Choice> {
    let mut choice = None;
    let resp = egui::Modal::new(Id::unique(id)).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.set_min_width(300.0);
        ui.heading(title);
        ui.add_space(8.0);
        body(ui);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = ui.add_enabled_ui(confirm_enabled, |ui| {
                    if danger { danger_button(ui, confirm) } else { primary_button(ui, confirm) }
                });
                if ok.inner.clicked() {
                    choice = Some(Choice::Confirm);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
    });
    if choice.is_none() && resp.should_close() {
        choice = Some(Choice::Cancel);
    }
    choice
}

/// Paint a horizontal meter.
pub fn meter(ui: &mut Ui, size: Vec2, fraction: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, size.y / 2.0, theme::RAISED);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    if fill.width() > 0.5 {
        p.rect_filled(fill, size.y / 2.0, color);
    }
    resp
}
