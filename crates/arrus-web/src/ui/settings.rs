//! Personal settings: UI colours, the formation-switch animation, and the
//! nickname (changeable once a month).

use arrus_api::{MAX_GLIDE_SECS, Me, UpdateMe, UserPrefs};
use egui::{Color32, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use super::widgets::{self, Choice};
use super::{App, net};
use crate::theme::{self, Palette};
use crate::web;

#[derive(Default)]
pub struct SettingsUi {
    open: bool,
    /// The settings being edited (shown live; saved on Done).
    pub draft: Option<UserPrefs>,
    bg_hex: String,
    accent_hex: String,
    saving: bool,
    nickname: String,
    password: String,
    renaming: bool,
    error: Option<String>,
}

/// Background presets: all dark, so the light text stays readable.
const BACKGROUNDS: [(u8, u8, u8); 8] = [
    (0x0B, 0x0B, 0x0C),
    (0x00, 0x00, 0x00),
    (0x14, 0x14, 0x14),
    (0x0A, 0x10, 0x1C),
    (0x0B, 0x15, 0x12),
    (0x14, 0x0D, 0x1C),
    (0x1A, 0x0F, 0x10),
    (0x16, 0x13, 0x0C),
];

const ACCENTS: [(u8, u8, u8); 8] = [
    (0x92, 0x66, 0xF8),
    (0x3A, 0x86, 0xFF),
    (0x00, 0xB8, 0xC4),
    (0x2E, 0xC2, 0x7E),
    (0xF5, 0xA5, 0x24),
    (0xF0, 0x6A, 0x3C),
    (0xE0, 0x47, 0x9E),
    (0xC8, 0xC8, 0xD0),
];

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([b(0)?, b(2)?, b(4)?])
}

/// The palette for a person's settings.
pub fn palette_of(p: &UserPrefs) -> Palette {
    Palette::new(
        p.background.map_or(theme::DEFAULT_BG, rgb),
        p.accent.map_or(theme::DEFAULT_ACCENT, rgb),
    )
}

/// The settings in effect: the open dialog's draft (a live preview), else the saved ones.
pub fn current_prefs(app: &App) -> Option<&UserPrefs> {
    app.settings.draft.as_ref().or(app.me.as_ref().map(|m| &m.prefs))
}

pub fn open(app: &mut App) {
    let Some(me) = &app.me else { return };
    let p = me.prefs.clone();
    let pal = palette_of(&p);
    app.settings = SettingsUi {
        open: true,
        bg_hex: hex(pal.bg),
        accent_hex: hex(pal.accent),
        draft: Some(p),
        nickname: me.username.clone(),
        ..Default::default()
    };
}

fn save_prefs(app: &mut App, prefs: UserPrefs) {
    app.settings.saving = true;
    app.settings.error = None;
    app.net.spawn(async move {
        let r = net::send::<_, UserPrefs>("PUT", "/api/me/prefs", &prefs).await;
        move |app: &mut App| {
            app.settings.saving = false;
            match r {
                Ok(saved) => {
                    if let Some(me) = &mut app.me {
                        me.prefs = saved;
                    }
                    app.settings.open = false;
                    app.settings.draft = None;
                    app.toast("Settings saved");
                }
                Err(net::ApiError::Unauthorized) => web::navigate("/login"),
                Err(e) => app.settings.error = Some(e.to_string()),
            }
        }
    });
}

fn rename(app: &mut App, nickname: String, password: String) {
    app.settings.renaming = true;
    app.settings.error = None;
    let body = UpdateMe { current_password: password, nickname: Some(nickname), ..Default::default() };
    app.net.spawn(async move {
        let r = net::send::<_, Me>("PATCH", "/api/me", &body).await;
        move |app: &mut App| {
            app.settings.renaming = false;
            match r {
                Ok(me) => {
                    app.settings.nickname = me.username.clone();
                    app.settings.password.clear();
                    // The colours being previewed stay in the draft.
                    app.me = Some(me);
                    app.toast("Nickname changed");
                }
                Err(net::ApiError::Unauthorized) => web::navigate("/login"),
                Err(e) => app.settings.error = Some(e.to_string()),
            }
        }
    });
}

/// A row of colour swatches; returns the one clicked.
fn swatches(ui: &mut Ui, colors: &[(u8, u8, u8)], current: Color32) -> Option<[u8; 3]> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for &(r, g, b) in colors {
            let c = Color32::from_rgb(r, g, b);
            let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(rect, 6.0, c);
            let selected = c == current;
            let stroke = if selected {
                Stroke::new(2.0, Color32::WHITE)
            } else if resp.hovered() {
                Stroke::new(1.5, theme::TEXT_WEAK)
            } else {
                Stroke::new(1.0, theme::border())
            };
            p.rect_stroke(rect, 6.0, stroke, StrokeKind::Inside);
            if resp.on_hover_text(hex(c)).clicked() {
                picked = Some([r, g, b]);
            }
        }
    });
    picked
}

/// A `#RRGGBB` field; returns a valid colour once typed.
fn hex_field(ui: &mut Ui, text: &mut String) -> Option<[u8; 3]> {
    let r = ui.add(egui::TextEdit::singleline(text).desired_width(80.0).hint_text("#RRGGBB"));
    if r.changed() { parse_hex(text) } else { None }
}

pub fn dialog(app: &mut App, ctx: &egui::Context) {
    if !app.settings.open {
        return;
    }
    let Some(me) = app.me.clone() else { return };
    let Some(mut draft) = app.settings.draft.clone() else { return };
    let pal = palette_of(&draft);
    let now = web::now_secs();
    let rename_wait = me.nickname_change_from.filter(|&t| t > now);
    let s = &mut app.settings;
    let mut do_rename = None;
    let busy = s.saving;
    let c = widgets::dialog(ctx, "user_settings", "Settings", "Done", false, !busy, |ui| {
        ui.set_min_width(360.0);

        widgets::section(ui, "Colours");
        ui.label("Background");
        if let Some(c) = swatches(ui, &BACKGROUNDS, pal.bg) {
            draft.background = Some(c);
            s.bg_hex = hex(rgb(c));
        }
        ui.horizontal(|ui| {
            if let Some(c) = hex_field(ui, &mut s.bg_hex) {
                draft.background = Some(c);
            }
            if draft.background.is_some_and(|b| theme::clamp_background(rgb(b)) != rgb(b)) {
                ui.label(RichText::new("darkened to keep text readable").small().color(theme::TEXT_FAINT));
            }
        });
        ui.add_space(6.0);
        ui.label("Accent");
        if let Some(c) = swatches(ui, &ACCENTS, pal.accent) {
            draft.accent = Some(c);
            s.accent_hex = hex(rgb(c));
        }
        ui.horizontal(|ui| {
            if let Some(c) = hex_field(ui, &mut s.accent_hex) {
                draft.accent = Some(c);
            }
            if draft.accent.is_some_and(|a| theme::clamp_accent(rgb(a)) != rgb(a)) {
                ui.label(RichText::new("lightened to stay readable").small().color(theme::TEXT_FAINT));
            }
        });
        if (draft.background.is_some() || draft.accent.is_some()) && ui.button("Reset to the default colours").clicked() {
            draft.background = None;
            draft.accent = None;
            s.bg_hex = hex(theme::DEFAULT_BG);
            s.accent_hex = hex(theme::DEFAULT_ACCENT);
        }

        widgets::section(ui, "Animation");
        ui.horizontal(|ui| {
            ui.label("Switching formations");
            ui.add(
                egui::Slider::new(&mut draft.glide_secs, 0.0..=MAX_GLIDE_SECS)
                    .step_by(0.05)
                    .fixed_decimals(2)
                    .suffix(" s"),
            );
        });
        ui.label(
            RichText::new(if draft.glide_secs <= 0.0 {
                "No animation: the dancers jump to the next formation."
            } else {
                "How long the dancers take to move when you go to another formation by hand."
            })
            .small()
            .color(theme::TEXT_FAINT),
        );

        if me.account_id.is_some() {
            widgets::section(ui, "Nickname");
            match rename_wait {
                Some(t) => {
                    ui.label(RichText::new(&me.username).strong());
                    ui.label(
                        RichText::new(format!("You can change it once a month: again from {}.", web::local_datetime(t)))
                            .small()
                            .color(theme::TEXT_WEAK),
                    );
                }
                None => {
                    egui::Grid::new("nick_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Nickname");
                        ui.add(egui::TextEdit::singleline(&mut s.nickname).desired_width(200.0));
                        ui.end_row();
                        ui.label("Password");
                        ui.add(
                            egui::TextEdit::singleline(&mut s.password)
                                .password(true)
                                .hint_text("to confirm")
                                .desired_width(200.0),
                        );
                        ui.end_row();
                    });
                    let new = s.nickname.trim();
                    let ok = !new.is_empty() && new != me.username && !s.password.is_empty() && !s.renaming;
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ok, egui::Button::new("Change nickname")).clicked() {
                            do_rename = Some((new.to_owned(), s.password.clone()));
                        }
                        if s.renaming {
                            ui.spinner();
                        }
                    });
                    ui.label(
                        RichText::new("You can change it once a month.").small().color(theme::TEXT_FAINT),
                    );
                }
            }
        }

        if let Some(e) = &s.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(theme::ERROR));
        }
    });
    app.settings.draft = Some(draft.clone());
    if let Some((n, p)) = do_rename {
        rename(app, n, p);
    }
    match c {
        Some(Choice::Confirm) => save_prefs(app, draft.cleaned()),
        Some(Choice::Cancel) => {
            app.settings.open = false;
            app.settings.draft = None;
        }
        None => {}
    }
}
