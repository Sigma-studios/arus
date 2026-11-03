//! The application: session, top bar, screens, toasts and alerts.

mod admin;
mod editor;
mod inspector;
mod library;
mod net;
mod stage;
mod timeline;
mod widgets;

use arrus_api::{Me, Role, Storage, Team, TeamId};
use egui::{Align, Align2, Frame, Id, Layout, Margin, RichText, Stroke, Ui, vec2};

use crate::util::{self, Level};
use crate::{theme, web};
pub use net::{ApiError, Net};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Library,
    Admin,
    Editor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    until: f64,
}

pub struct Alert {
    pub title: String,
    pub text: String,
}

pub struct App {
    pub net: Net,
    pub me: Option<Me>,
    boot_error: Option<String>,
    pub storage: Option<Storage>,
    /// Admin only.
    pub teams: Vec<Team>,
    /// The team shown (the member's own, or the admin's choice).
    pub team: Option<TeamId>,
    pub screen: Screen,
    pub library: library::Library,
    pub admin: admin::Admin,
    pub editor: Option<Box<editor::Editor>>,
    /// Choreography being fetched for the editor.
    pub opening: Option<i64>,
    toasts: Vec<Toast>,
    pub alert: Option<Alert>,
    /// Coarse pointer (phone / tablet).
    touch: bool,
    /// Touch device or narrow viewport: phone layout.
    pub compact: bool,
    styled_for: Option<bool>,
    /// Screen to go to once the user confirms leaving unsaved changes.
    leave_to: Option<Screen>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let net = Net::new(cc.egui_ctx.clone());
        let touch = web::coarse_pointer();
        let app = Self {
            net,
            me: None,
            boot_error: None,
            storage: None,
            teams: Vec::new(),
            team: None,
            screen: Screen::Library,
            library: Default::default(),
            admin: Default::default(),
            editor: None,
            opening: None,
            toasts: Vec::new(),
            alert: None,
            touch,
            compact: touch,
            styled_for: None,
            leave_to: None,
        };
        app.net.spawn(async {
            let r = net::get::<Me>("/api/me").await;
            move |app: &mut App| app.on_me(r)
        });
        app
    }

    fn on_me(&mut self, r: net::ApiResult<Me>) {
        match r {
            Ok(me) => {
                let admin = me.role == Role::Admin;
                self.team = me.team.as_ref().map(|t| t.id);
                self.me = Some(me);
                self.refresh_storage();
                if admin {
                    self.load_teams();
                } else {
                    library::reload(self);
                }
                if let Some(id) = web::hash_choreography() {
                    editor::open(self, id);
                } else if admin && web::hash_is_admin() {
                    self.go(Screen::Admin);
                }
            }
            Err(ApiError::Unauthorized) => web::navigate("/login"),
            Err(e) => self.boot_error = Some(e.to_string()),
        }
    }

    pub fn is_admin(&self) -> bool {
        self.me.as_ref().is_some_and(|m| m.role == Role::Admin)
    }

    /// May edit (has write access), on any device.
    pub fn can_edit(&self) -> bool {
        self.me.as_ref().is_some_and(|m| m.can_write)
    }

    pub fn team_name(&self) -> Option<String> {
        if let Some(t) = self.me.as_ref().and_then(|m| m.team.as_ref()) {
            return Some(t.name.clone());
        }
        let id = self.team?;
        self.teams.iter().find(|t| t.id == id).map(|t| t.name.clone())
    }

    pub fn load_teams(&mut self) {
        self.net.spawn(async {
            let r = net::get::<Vec<Team>>("/api/teams").await;
            move |app: &mut App| match r {
                Ok(teams) => {
                    app.teams = teams;
                    if app.team.is_none_or(|id| !app.teams.iter().any(|t| t.id == id)) {
                        let first = app.teams.first().map(|t| t.id);
                        app.set_team(first);
                    }
                }
                Err(e) => app.fail(e),
            }
        });
    }

    pub fn set_team(&mut self, id: Option<TeamId>) {
        let changed = self.team != id;
        self.team = id;
        if changed {
            self.library.items.clear();
            self.library.loaded_for = None;
            self.admin.members.clear();
            self.admin.members_for = None;
        }
        library::reload(self);
        if self.is_admin() {
            admin::load_members(self);
        }
    }

    pub fn refresh_storage(&mut self) {
        self.net.spawn(async {
            let r = net::get::<Storage>("/api/storage").await;
            move |app: &mut App| match r {
                Ok(s) => app.storage = Some(s),
                Err(e) => app.fail(e),
            }
        });
    }

    /// Report an API failure the standard way.
    pub fn fail(&mut self, e: ApiError) {
        match e {
            ApiError::Unauthorized => web::navigate("/login"),
            ApiError::StorageFull => {
                self.alert = Some(Alert {
                    title: "Storage is full".into(),
                    text: "The server has no room left for choreographies and music. Delete old \
                           choreographies or music tracks (or ask the admin to), then try again."
                        .into(),
                });
                self.refresh_storage();
            }
            e => self.toast_error(e.to_string()),
        }
    }

    fn now(&self) -> f64 {
        self.net.ctx().input(|i| i.time)
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let until = self.now() + 4.0;
        self.toasts.push(Toast { text: text.into(), kind: ToastKind::Success, until });
    }

    pub fn toast_error(&mut self, text: impl Into<String>) {
        let until = self.now() + 8.0;
        self.toasts.push(Toast { text: text.into(), kind: ToastKind::Error, until });
    }

    /// Switch screens, asking first if the editor has unsaved changes.
    pub fn go(&mut self, to: Screen) {
        if self.screen == Screen::Editor && to != Screen::Editor {
            if self.editor.as_ref().is_some_and(|e| e.dirty) {
                self.leave_to = Some(to);
                return;
            }
            self.close_editor();
        }
        self.screen = to;
        web::set_hash_admin(to == Screen::Admin);
        match to {
            Screen::Library => library::reload(self),
            Screen::Admin => {
                self.load_teams();
                admin::load_members(self);
                admin::load_alerts(self);
            }
            Screen::Editor => {}
        }
    }

    pub fn close_editor(&mut self) {
        self.editor = None;
        self.opening = None;
        web::set_hash_choreography(None);
        web::set_unsaved(false);
        if self.screen == Screen::Editor {
            self.screen = Screen::Library;
            library::reload(self);
        }
    }

    fn logout(&mut self) {
        self.net.spawn(async {
            let _ = net::send_empty("POST", "/logout").await;
            |_: &mut App| web::navigate("/login")
        });
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let compact = self.compact;
        egui::Panel::top("top_bar")
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(14, 8)))
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("arus").size(if compact { 20.0 } else { 22.0 }).strong().color(theme::ACCENT));
                    ui.add_space(if compact { 2.0 } else { 14.0 });
                    let in_editor = self.screen == Screen::Editor;
                    // On compact layouts the library is the only screen (the
                    // editor has its own back button), so skip the nav button.
                    if !compact && ui.add(egui::Button::selectable(self.screen == Screen::Library, "Library")).clicked() {
                        self.go(Screen::Library);
                    }
                    if self.is_admin()
                        && !compact
                        && ui.add(egui::Button::selectable(self.screen == Screen::Admin, "Teams & members")).clicked()
                    {
                        self.go(Screen::Admin);
                    }
                    if !compact {
                        ui.add_space(8.0);
                    }
                    if self.is_admin() {
                        let current = self.team_name().unwrap_or_else(|| "No team".into());
                        let mut pick = None;
                        ui.add_enabled_ui(!in_editor, |ui| {
                            egui::ComboBox::from_id_salt("team_select")
                                .selected_text(if compact { RichText::new(current) } else { RichText::new(format!("Team: {current}")) })
                                .truncate()
                                .width(if compact { 110.0 } else { 200.0 })
                                .show_ui(ui, |ui| {
                                    for t in &self.teams {
                                        if ui.add(egui::Button::selectable(Some(t.id) == self.team, &t.name)).clicked() {
                                            pick = Some(t.id);
                                        }
                                    }
                                    if self.teams.is_empty() {
                                        ui.label(RichText::new("No teams yet").color(theme::TEXT_WEAK));
                                    }
                                })
                                .response
                                .on_disabled_hover_text("Leave the editor to switch teams");
                        });
                        if let Some(id) = pick {
                            self.set_team(Some(id));
                        }
                    } else if let Some(name) = self.team_name()
                        && !compact
                    {
                        ui.label(RichText::new(name).color(theme::TEXT_WEAK));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if compact {
                            self.user_menu(ui);
                            return;
                        }
                        if ui.button("Log out").clicked() {
                            self.logout();
                        }
                        if let Some(me) = &self.me {
                            if !compact {
                                ui.label(RichText::new(&me.username).strong());
                            }
                            if me.role == Role::Admin {
                                widgets::badge(ui, "ADMIN", theme::SELECT);
                            } else if !me.can_write {
                                widgets::badge(ui, "VIEWER", theme::TEXT_WEAK);
                            }
                        }
                        if !compact && let Some(s) = &self.storage {
                            ui.add_space(10.0);
                            storage_meter(ui, s);
                        }
                    });
                });
            });
    }

    /// Compact layouts: user, team, storage and logout behind one button.
    fn user_menu(&mut self, ui: &mut Ui) {
        let Some(me) = self.me.clone() else { return };
        let mut logout = false;
        ui.menu_button(RichText::new(&me.username).strong(), |ui| {
            ui.set_min_width(220.0);
            ui.label(RichText::new(&me.username).strong());
            let role = match (me.role, me.can_write) {
                (Role::Admin, _) => "Admin",
                (_, true) => "Member (can edit on a computer)",
                (_, false) => "Viewer",
            };
            ui.label(RichText::new(role).small().color(theme::TEXT_WEAK));
            if let Some(t) = self.team_name() {
                ui.label(RichText::new(format!("Team: {t}")).small().color(theme::TEXT_WEAK));
            }
            if let Some(s) = &self.storage {
                ui.add_space(6.0);
                storage_meter(ui, s);
            }
            ui.separator();
            if ui.button("Log out").clicked() {
                logout = true;
                ui.close();
            }
        });
        if logout {
            self.logout();
        }
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        let now = self.now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let mut dismiss = None;
        egui::Area::new(Id::unique("toasts"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(egui::Order::Tooltip)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(480.0);
                for (i, t) in self.toasts.iter().enumerate() {
                    let col = match t.kind {
                        ToastKind::Success => theme::OK,
                        ToastKind::Error => theme::ERROR,
                    };
                    let r = Frame::new()
                        .fill(theme::RAISED)
                        .stroke(Stroke::new(1.0, col.gamma_multiply(0.7)))
                        .corner_radius(8)
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").color(col));
                                ui.label(RichText::new(&t.text).color(theme::TEXT));
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if r.clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

    fn alerts_ui(&mut self, ctx: &egui::Context) {
        if let Some(a) = &self.alert {
            let (title, text) = (a.title.clone(), a.text.clone());
            let r = egui::Modal::new(Id::unique("alert")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading(title);
                ui.add_space(8.0);
                ui.label(text);
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| widgets::primary_button(ui, "OK").clicked())
                    .inner
            });
            if r.inner || r.should_close() {
                self.alert = None;
            }
        }
        if let Some(to) = self.leave_to {
            match widgets::dialog(ctx, "leave_unsaved", "Discard unsaved changes?", "Discard and leave", true, true, |ui| {
                ui.label("This choreography has changes that are not saved yet.");
            }) {
                Some(widgets::Choice::Confirm) => {
                    self.leave_to = None;
                    if let Some(e) = &mut self.editor {
                        e.dirty = false;
                    }
                    self.go(to);
                }
                Some(widgets::Choice::Cancel) => self.leave_to = None,
                None => {}
            }
        }
    }
}

fn storage_meter(ui: &mut Ui, s: &Storage) {
    let frac = if s.limit > 0 { s.used as f32 / s.limit as f32 } else { 1.0 };
    let col = match util::storage_level(s.used, s.limit) {
        Level::Ok => theme::OK,
        Level::Warn => theme::WARN,
        Level::Critical => theme::ERROR,
    };
    let text = format!("{} of {}", util::fmt_bytes(s.used), util::fmt_bytes(s.limit));
    let r = ui
        .horizontal(|ui| {
            ui.label(RichText::new(&text).small().color(if col == theme::OK { theme::TEXT_WEAK } else { col }));
            widgets::meter(ui, vec2(90.0, 7.0), frac, col);
        })
        .response;
    r.on_hover_ui(|ui| {
        ui.label(format!("Storage used: {text} ({:.0} %)", frac * 100.0));
        for t in &s.teams {
            ui.label(format!("{}: {}", t.name, util::fmt_bytes(t.bytes)));
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        for h in self.net.drain() {
            h(self);
        }

        let narrow = ctx.content_rect().width() < 900.0;
        self.compact = self.touch || narrow;
        if self.styled_for != Some(self.touch) {
            theme::apply(&ctx, self.touch);
            self.styled_for = Some(self.touch);
        }

        let bg = Frame::new().fill(theme::PANEL);
        if self.me.is_none() {
            egui::CentralPanel::default().frame(bg).show(ui, |ui| {
                ui.centered_and_justified(|ui| match &self.boot_error {
                    Some(e) => {
                        ui.label(RichText::new(format!("Could not start arus: {e}")).color(theme::ERROR));
                    }
                    None => {
                        ui.spinner();
                    }
                });
            });
            return;
        }

        self.top_bar(ui);
        match self.screen {
            Screen::Library => library::show(self, ui),
            Screen::Admin => admin::show(self, ui),
            Screen::Editor => {
                if let Some(mut ed) = self.editor.take() {
                    editor::show(&mut ed, self, ui);
                    if self.editor.is_none() && self.screen == Screen::Editor {
                        self.editor = Some(ed);
                    }
                } else {
                    egui::CentralPanel::default().frame(bg).show(ui, |ui| {
                        ui.centered_and_justified(|ui| ui.spinner());
                    });
                }
            }
        }
        self.toasts_ui(&ctx);
        self.alerts_ui(&ctx);
        web::set_unsaved(self.editor.as_ref().is_some_and(|e| e.dirty));
    }
}
