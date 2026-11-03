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
