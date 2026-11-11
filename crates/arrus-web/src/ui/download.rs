//! Download screen: the Windows app (a window around this site, see `desktop/`).

use egui::{Frame, Margin, RichText, ScrollArea, Ui};

use super::{App, widgets};
use crate::theme;

/// The newest Windows build, attached to the repository's latest GitHub release.
pub const WINDOWS_ZIP: &str = "https://github.com/Sigma-studios/arus/releases/latest/download/arus-windows.zip";

pub fn show(app: &mut App, ui: &mut Ui) {
    let compact = app.compact;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(720.0);
                ui.heading("Download");
                ui.add_space(10.0);
                widgets::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("arus for Windows").size(18.0).strong());
                    ui.label(
                        RichText::new(
                            "arus in its own window, with its own taskbar icon. You stay logged in, and it always \
                             shows the latest version of arus, so you never need to download it again.",
                        )
                        .color(theme::TEXT_WEAK),
                    );
                    ui.add_space(10.0);
                    if widgets::primary_button(ui, "⬇  Download for Windows (zip, 12 MB)").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::same_tab(WINDOWS_ZIP));
                    }
                    ui.add_space(12.0);
                    widgets::section(ui, "Install");
                    for (i, step) in [
                        "Open the downloaded arus-windows.zip and choose \"Extract all\".",
                        "Move the extracted folder where you want to keep it, e.g. Documents.",
                        "Open arus.exe in that folder. Keep the other files next to it.",
                        "Optional: right-click arus.exe → \"Pin to taskbar\" or \"Send to → Desktop (create shortcut)\".",
                    ]
                    .iter()
                    .enumerate()
                    {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{}.", i + 1)).strong().color(theme::accent()));
                            ui.label(*step);
                        });
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "Windows may warn that the app is from an unknown publisher (it isn't signed): click \
                             \"More info\" → \"Run anyway\". Needs Windows 10 or 11.",
                        )
                        .small()
                        .color(theme::TEXT_FAINT),
                    );
                });
            });
        });
}
