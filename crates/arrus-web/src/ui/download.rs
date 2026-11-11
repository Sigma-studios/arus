//! Download screen: the arus apps (app/ in the repository) for every system.
//! The visitor's own system comes first, with its install steps open.

use egui::{Frame, Margin, RichText, ScrollArea, Ui};

use super::{App, widgets};
use crate::{theme, web};

/// Files attached to the repository's latest GitHub release (built by
/// .github/workflows/apps.yml).
const RELEASE: &str = "https://github.com/Sigma-studios/arus/releases/latest/download/";

/// The App Store page, once Apple has approved the app.
const APP_STORE: Option<&str> = None;

struct Platform {
    os: &'static str,
    name: &'static str,
    blurb: &'static str,
    /// File in the latest release; `None`: the store link.
    file: Option<&'static str>,
    button: &'static str,
    steps: &'static [&'static str],
    note: &'static str,
}

const PLATFORMS: &[Platform] = &[
    Platform {
        os: "windows",
        name: "Windows",
        blurb: "Windows 10 and 11.",
        file: Some("arus-windows.zip"),
        button: "Download for Windows",
        steps: &[
            "Open the downloaded arus-windows.zip and choose \"Extract all\".",
            "Move the extracted folder where you want to keep it, e.g. Documents.",
            "Open arus.exe in that folder. Keep the other files next to it.",
            "Optional: right-click arus.exe → \"Pin to taskbar\".",
        ],
        note: "Windows may warn about an unknown publisher (the app isn't signed): \
               click \"More info\" → \"Run anyway\".",
    },
    Platform {
        os: "macos",
        name: "macOS",
        blurb: "macOS 12 (Monterey) or newer, Apple silicon and Intel.",
        file: Some("arus-macos.zip"),
        button: "Download for macOS",
        steps: &[
            "Open the downloaded arus-macos.zip (Safari may already have unzipped it).",
            "Drag arus.app into your Applications folder.",
            "Open arus. If macOS says it can't check the app: open System Settings → \
             Privacy & Security, scroll down and click \"Open Anyway\".",
        ],
        note: "The app isn't notarized by Apple yet, hence the one-time warning.",
    },
    Platform {
        os: "linux",
        name: "Linux",
        blurb: "64-bit (x86-64) with GTK 3 and WebKitGTK.",
        file: Some("arus-linux-x64.tar.gz"),
        button: "Download for Linux",
        steps: &[
            "Extract it: tar xzf arus-linux-x64.tar.gz",
            "Run ./arus/arus (keep the other files next to it).",
            "If it doesn't start, install WebKitGTK: sudo apt install libwebkit2gtk-4.1-0 \
             (Debian/Ubuntu) or sudo dnf install webkit2gtk4.1 (Fedora).",
        ],
        note: "",
    },
    Platform {
        os: "android",
        name: "Android",
        blurb: "Android 7 or newer.",
        file: Some("arus-android.apk"),
        button: "Download for Android",
        steps: &[
            "Open the downloaded arus-android.apk.",
            "If asked, allow your browser to install unknown apps, then tap \"Install\".",
            "Open arus from your home screen.",
        ],
        note: "The app comes from here, not from Google Play: Android asks once \
               whether you trust the source.",
    },
    Platform {
        os: "ios",
        name: "iPhone & iPad",
        blurb: "iOS 15 or newer, from the App Store.",
        file: None,
        button: "Open the App Store",
        steps: &[],
        note: "",
    },
];

pub fn show(app: &mut App, ui: &mut Ui) {
    let compact = app.compact;
    let here = web::device_os();
    let mut order: Vec<&Platform> = PLATFORMS.iter().filter(|p| p.os == here).collect();
    order.extend(PLATFORMS.iter().filter(|p| p.os != here));
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(720.0);
                ui.heading("Get the app");
                ui.label(
                    RichText::new(
                        "arus in its own window, with its own icon. You stay logged in, and the app always \
                         shows the latest version of arus, so you only install it once.",
                    )
                    .color(theme::TEXT_WEAK),
                );
                ui.add_space(12.0);
                for p in order {
                    card(ui, p, p.os == here);
                    ui.add_space(10.0);
                }
            });
        });
}

fn card(ui: &mut Ui, p: &Platform, this_device: bool) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::screen_header(
            ui,
            ui.available_width() < 420.0,
            |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(p.name).size(17.0).strong());
                        if this_device {
                            widgets::badge(ui, "THIS DEVICE", theme::accent());
                        }
                    });
                    ui.label(RichText::new(p.blurb).small().color(theme::TEXT_WEAK));
                });
            },
            |ui| match (p.file, APP_STORE) {
                (Some(file), _) => {
                    if widgets::primary_button(ui, &format!("⬇  {}", p.button)).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::same_tab(format!("{RELEASE}{file}")));
                    }
                }
                (None, Some(store)) => {
                    if widgets::primary_button(ui, p.button).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(store));
                    }
                }
                (None, None) => {
                    ui.add_enabled(false, egui::Button::new("Coming soon to the App Store"));
                }
            },
        );
        if !p.steps.is_empty() {
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("How to install").color(theme::TEXT_WEAK))
                .id_salt(("install", p.os))
                .default_open(this_device)
                .show(ui, |ui| {
                    for (i, step) in p.steps.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{}.", i + 1)).strong().color(theme::accent()));
                            ui.label(*step);
                        });
                    }
                    if !p.note.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(p.note).small().color(theme::TEXT_FAINT));
                    }
                });
        }
    });
}
