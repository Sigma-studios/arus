//! The application: session, top bar, screens, toasts and alerts.

mod account;
mod accounts;
mod admin;
mod editor;
mod inspector;
mod library;
mod live;
mod motion;
mod net;
mod stage;
mod team;
mod timeline;
mod widgets;

use arrus_api::{CreateTeam, FREE_CHOREOGRAPHIES, FREE_TEAMS, Me, MyTeam, Role, Storage, Team, TeamId, TeamRole};
use egui::{Align, Align2, Frame, Id, Layout, Margin, RichText, Stroke, Ui, vec2};

use crate::util::{self, Level};
use crate::{theme, web};
pub use net::{ApiError, Net};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Library,
    /// The current team: who is in it, share links.
    Team,
    /// Admin: every team, storage alerts.
    Admin,
    /// Admin: every account.
    Accounts,
    Editor,
}

impl Screen {
    /// Its URL fragment.
    fn hash(self) -> Option<&'static str> {
        match self {
            Screen::Team => Some("team"),
            Screen::Admin => Some("admin"),
            Screen::Accounts => Some("accounts"),
            Screen::Library | Screen::Editor => None,
        }
    }
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
    /// Admin only: every team.
    pub teams: Vec<Team>,
    /// The team shown.
    pub team: Option<TeamId>,
    pub screen: Screen,
    pub library: library::Library,
    pub admin: admin::Admin,
    pub accounts: accounts::Accounts,
    pub team_ui: team::TeamUi,
    pub account: account::AccountUi,
    /// "New team" dialog: the name typed, and whether it is being created.
    new_team: Option<(String, bool)>,
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
            accounts: Default::default(),
            team_ui: Default::default(),
            account: Default::default(),
            new_team: None,
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
                let wanted = web::hash_team();
                self.me = Some(me);
                self.refresh_storage();
                let mine = self.my_teams().iter().map(|t| t.id).collect::<Vec<_>>();
                let first = wanted.filter(|t| admin || mine.contains(t)).or_else(|| mine.first().copied());
                self.set_team(first);
                if admin {
                    self.load_teams();
                }
                if let Some(id) = web::hash_choreography() {
                    editor::open(self, id);
                } else {
                    match web::hash_screen().as_deref() {
                        Some("admin") if admin => self.go(Screen::Admin),
                        Some("accounts") if admin => self.go(Screen::Accounts),
                        Some("team") if self.team.is_some() => self.go(Screen::Team),
                        _ => web::set_hash_screen(None),
                    }
                }
            }
            Err(ApiError::Unauthorized) => web::navigate("/login"),
            Err(e) => self.boot_error = Some(e.to_string()),
        }
    }

    /// Fetch who is logged in again (teams, roles, limits changed).
    pub fn reload_me(&mut self) {
        self.net.spawn(async {
            let r = net::get::<Me>("/api/me").await;
            move |app: &mut App| match r {
                Ok(me) => {
                    app.me = Some(me);
                    let ok = app.team.is_some_and(|id| app.is_admin() || app.my_teams().iter().any(|t| t.id == id));
                    if !ok {
                        let first = app.my_teams().first().map(|t| t.id);
                        app.set_team(first);
                        if app.screen == Screen::Team && app.team.is_none() {
                            app.go(Screen::Library);
                        }
                    }
                }
                Err(e) => app.fail(e),
            }
        });
    }

    pub fn is_admin(&self) -> bool {
        self.me.as_ref().is_some_and(|m| m.role == Role::Admin)
    }

    pub fn my_teams(&self) -> &[MyTeam] {
        self.me.as_ref().map_or(&[], |m| m.teams.as_slice())
    }

    /// This account's role in the current team.
    pub fn my_role(&self) -> Option<TeamRole> {
        let id = self.team?;
        self.my_teams().iter().find(|t| t.id == id).map(|t| t.role)
    }

    /// May change the current team's choreographies.
    pub fn can_edit(&self) -> bool {
        self.is_admin() || self.my_role().is_some_and(TeamRole::can_write)
    }

    /// May decide who is in the current team, rename or delete it.
    pub fn can_manage(&self) -> bool {
        self.team.is_some() && (self.is_admin() || self.my_role() == Some(TeamRole::Owner))
    }

    pub fn team_name(&self) -> Option<String> {
        let id = self.team?;
        if let Some(t) = self.my_teams().iter().find(|t| t.id == id) {
            return Some(t.name.clone());
        }
        self.teams.iter().find(|t| t.id == id).map(|t| t.name.clone())
    }

    /// The current team's choreography limit (`None`: unlimited).
    pub fn choreography_limit(&self) -> Option<u32> {
        let id = self.team?;
        match self.my_teams().iter().find(|t| t.id == id) {
            Some(t) => t.choreography_limit,
            None => self.teams.iter().find(|t| t.id == id).and_then(|t| t.choreography_limit),
        }
    }

    /// Teams to switch between: all of them for the admin.
    fn switchable_teams(&self) -> Vec<(TeamId, String)> {
        if self.is_admin() {
            let mut v: Vec<(TeamId, String)> = self.teams.iter().map(|t| (t.id, t.name.clone())).collect();
            for t in self.my_teams() {
                if !v.iter().any(|(id, _)| *id == t.id) {
                    v.push((t.id, t.name.clone()));
                }
            }
            v
        } else {
            self.my_teams().iter().map(|t| (t.id, t.name.clone())).collect()
        }
    }

    pub fn load_teams(&mut self) {
        self.net.spawn(async {
            let r = net::get::<Vec<Team>>("/api/teams").await;
            move |app: &mut App| match r {
                Ok(teams) => {
                    app.teams = teams;
                    if app.team.is_none_or(|id| !app.teams.iter().any(|t| t.id == id)) {
                        let first = app.my_teams().first().map(|t| t.id).or_else(|| app.teams.first().map(|t| t.id));
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
            self.team_ui = Default::default();
        }
        library::reload(self);
        if matches!(self.screen, Screen::Team | Screen::Admin) {
            team::load(self);
        }
    }

    /// Create a team (owned by this account), then switch to it.
    pub fn create_team(&mut self, name: String) {
        self.net.spawn(async move {
            let r = net::send::<_, Team>("POST", "/api/teams", &CreateTeam { name }).await;
            move |app: &mut App| {
                app.new_team = None;
                match r {
                    Ok(t) => {
                        app.toast(format!("Team “{}” created", t.name));
                        let id = t.id;
                        if app.is_admin() {
                            app.teams.push(t);
                            app.load_teams();
                        }
                        app.reload_me();
                        app.set_team(Some(id));
                    }
                    Err(e) => app.fail(e),
                }
            }
        });
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
            ApiError::Http { code: Some(c), .. } if c.starts_with("limit_") => {
                let text = if c == "limit_teams" {
                    format!("A free account can create {FREE_TEAMS} team. Premium accounts can create as many as they like.")
                } else {
                    format!(
                        "A team of a free account holds {FREE_CHOREOGRAPHIES} choreographies. Delete one first, \
                         or ask for premium (unlimited)."
                    )
                };
                self.alert = Some(Alert { title: "Free account limit".into(), text });
                self.reload_me();
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
        web::set_hash_screen(to.hash());
        match to {
            Screen::Library => library::reload(self),
            Screen::Team => team::load(self),
            Screen::Admin => {
                self.load_teams();
                team::load(self);
                admin::load_alerts(self);
            }
            Screen::Accounts => accounts::load(self),
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
                    // editor has its own back button); the rest is in the user menu.
                    if !compact {
                        if ui.add(egui::Button::selectable(self.screen == Screen::Library, "Library")).clicked() {
                            self.go(Screen::Library);
                        }
                        if self.team.is_some()
                            && ui.add(egui::Button::selectable(self.screen == Screen::Team, "Team")).on_hover_text("Who is in this team, share links").clicked()
                        {
                            self.go(Screen::Team);
                        }
                        if self.is_admin() {
                            if ui.add(egui::Button::selectable(self.screen == Screen::Admin, "All teams")).clicked() {
                                self.go(Screen::Admin);
                            }
                            if ui.add(egui::Button::selectable(self.screen == Screen::Accounts, "Accounts")).clicked() {
                                self.go(Screen::Accounts);
                            }
                        }
                        ui.add_space(8.0);
                    }
                    self.team_switcher(ui, in_editor);

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.user_menu(ui);
                        if compact {
                            widgets::fullscreen_button(ui);
                            return;
                        }
                        if let Some(me) = &self.me {
                            if me.role == Role::Admin {
                                widgets::badge(ui, "ADMIN", theme::SELECT);
                            } else if me.premium {
                                widgets::badge(ui, "PREMIUM", theme::ACCENT);
                            }
                            if self.team.is_some() && !self.can_edit() {
                                widgets::badge(ui, "GUEST", theme::TEXT_WEAK);
                            }
                        }
                        if let Some(s) = &self.storage {
                            ui.add_space(10.0);
                            storage_meter(ui, s);
                        }
                    });
                });
            });
    }

    fn team_switcher(&mut self, ui: &mut Ui, in_editor: bool) {
        let compact = self.compact;
        let teams = self.switchable_teams();
        let can_create = self.me.as_ref().is_some_and(|m| m.can_create_team);
        if teams.is_empty() && !can_create {
            return;
        }
        let current = self.team_name().unwrap_or_else(|| "No team".into());
        let mut pick = None;
        let mut new_team = false;
        ui.add_enabled_ui(!in_editor, |ui| {
            egui::ComboBox::from_id_salt("team_select")
                .selected_text(if compact { RichText::new(current) } else { RichText::new(format!("Team: {current}")) })
                .truncate()
                .width(if compact { 110.0 } else { 200.0 })
                .show_ui(ui, |ui| {
                    for (id, name) in &teams {
                        if ui.add(egui::Button::selectable(Some(*id) == self.team, name)).clicked() {
                            pick = Some(*id);
                        }
                    }
                    if teams.is_empty() {
                        ui.label(RichText::new("No teams yet").color(theme::TEXT_WEAK));
                    }
                    if can_create {
                        ui.separator();
                        if ui.button("+ New team…").clicked() {
                            new_team = true;
                        }
                    }
                })
                .response
                .on_disabled_hover_text("Leave the editor to switch teams");
        });
        if let Some(id) = pick {
            self.set_team(Some(id));
        }
        if new_team {
            self.new_team = Some((String::new(), false));
        }
    }

    /// Who is logged in, their account settings, and (on phones) the screens.
    fn user_menu(&mut self, ui: &mut Ui) {
        let Some(me) = self.me.clone() else { return };
        let mut logout = false;
        let mut goto = None;
        let compact = self.compact;
        ui.menu_button(RichText::new(&me.username).strong(), |ui| {
            ui.set_min_width(230.0);
            ui.label(RichText::new(&me.username).strong());
            let plan = match (me.role, me.premium) {
                (Role::Admin, _) => "Admin",
                (_, true) => "Premium account",
                (_, false) => "Free account",
            };
            ui.label(RichText::new(plan).small().color(theme::TEXT_WEAK));
            if let Some(e) = &me.email {
                ui.label(RichText::new(e).small().color(theme::TEXT_WEAK));
            }
            if let (Some(t), Some(role)) = (self.team_name(), self.my_role()) {
                ui.label(RichText::new(format!("{t}: {}", role.label())).small().color(theme::TEXT_WEAK));
            }
            if compact && let Some(s) = &self.storage {
                ui.add_space(6.0);
                storage_meter(ui, s);
            }
            ui.separator();
            if compact {
                if self.team.is_some() && ui.button("Team").clicked() {
                    goto = Some(Screen::Team);
                    ui.close();
                }
                if me.role == Role::Admin {
                    if ui.button("All teams").clicked() {
                        goto = Some(Screen::Admin);
                        ui.close();
                    }
                    if ui.button("Accounts").clicked() {
                        goto = Some(Screen::Accounts);
                        ui.close();
                    }
                }
            }
            if me.account_id.is_some() && ui.button("Account settings…").clicked() {
                account::open(self);
                ui.close();
            }
            if ui.button("Log out").clicked() {
                logout = true;
                ui.close();
            }
        });
        if let Some(to) = goto {
            self.go(to);
        }
        if logout {
            self.logout();
        }
    }

    fn new_team_dialog(&mut self, ctx: &egui::Context) {
        let Some((name, busy)) = &mut self.new_team else { return };
        let valid = !name.trim().is_empty() && !*busy;
        let mut enter = false;
        let c = widgets::dialog(ctx, "new_team", "New team", "Create", false, valid, |ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. Unicup").desired_width(f32::INFINITY));
            if !r.has_focus() && name.is_empty() {
                r.request_focus();
            }
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.label(RichText::new("You are its owner: add people by nickname or with a share link.").small().color(theme::TEXT_WEAK));
        });
        let name = name.trim().to_owned();
        match c {
            Some(widgets::Choice::Confirm) => {
                self.new_team = Some((name.clone(), true));
                self.create_team(name);
            }
            None if enter && valid => {
                self.new_team = Some((name.clone(), true));
                self.create_team(name);
            }
            Some(widgets::Choice::Cancel) => self.new_team = None,
            None => {}
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

        // Phones: the editor's own bar (with its back button) is enough; the
        // app header would only take room from the stage.
        let in_phone_editor = self.compact && self.screen == Screen::Editor && self.editor.is_some();
        if !in_phone_editor {
            self.top_bar(ui);
        }
        match self.screen {
            Screen::Library => library::show(self, ui),
            Screen::Team => team::show(self, ui),
            Screen::Admin => admin::show(self, ui),
            Screen::Accounts => accounts::show(self, ui),
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
        self.new_team_dialog(&ctx);
        account::dialog(self, &ctx);
        self.toasts_ui(&ctx);
        self.alerts_ui(&ctx);
        web::set_unsaved(self.editor.as_ref().is_some_and(|e| e.dirty));
    }
}
