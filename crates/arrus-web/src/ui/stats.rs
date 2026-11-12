//! Admin: statistics of the whole site (activity, sign-ups, storage), every
//! account sortable by usage, and one account in detail.

use arrus_api::{AccountDetailStats, AccountId, AccountStats, DayStats, SiteStats, TeamRole};
use egui::{Align2, Color32, FontId, Frame, Margin, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};

use super::widgets;
use super::{App, net};
use crate::util::{self, Agg, bucket_size, bucketed, nice_max};
use crate::theme;

const WINDOWS: [(u32, &str); 5] = [(7, "7 days"), (30, "30 days"), (90, "90 days"), (365, "1 year"), (730, "2 years")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Usage,
    TotalUsage,
    LastActive,
    Edits,
    Choreographies,
    Storage,
    Teams,
    Joined,
    Name,
}

impl Sort {
    const ALL: [Sort; 9] = [
        Sort::Usage,
        Sort::TotalUsage,
        Sort::LastActive,
        Sort::Edits,
        Sort::Choreographies,
        Sort::Storage,
        Sort::Teams,
        Sort::Joined,
        Sort::Name,
    ];

    fn label(self) -> &'static str {
        match self {
            Sort::Usage => "Time used (period)",
            Sort::TotalUsage => "Time used (all time)",
            Sort::LastActive => "Last active",
            Sort::Edits => "Edits (period)",
            Sort::Choreographies => "Choreographies",
            Sort::Storage => "Storage",
            Sort::Teams => "Teams",
            Sort::Joined => "Joined",
            Sort::Name => "Name (A–Z)",
        }
    }

    /// Names ascend by default, everything else descends (biggest first).
    fn natural_desc(self) -> bool {
        self != Sort::Name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Free,
    Premium,
    Admins,
    /// No activity in the period.
    Inactive,
}

impl Filter {
    const ALL: [Filter; 5] = [Filter::All, Filter::Free, Filter::Premium, Filter::Admins, Filter::Inactive];

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Free => "Free",
            Filter::Premium => "Premium",
            Filter::Admins => "Admins",
            Filter::Inactive => "Inactive",
        }
    }

    fn keeps(self, a: &AccountStats) -> bool {
        match self {
            Filter::All => true,
            Filter::Free => !a.is_premium && !a.is_admin,
            Filter::Premium => a.is_premium,
            Filter::Admins => a.is_admin,
            Filter::Inactive => a.active_days == 0,
        }
    }
}

pub struct Stats {
    days: u32,
    site: Option<SiteStats>,
    loading: bool,
    search: String,
    sort: Sort,
    desc: bool,
    filter: Filter,
    /// The account shown in detail, and its figures once loaded.
    selected: Option<AccountId>,
    detail: Option<AccountDetailStats>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            days: 30,
            site: None,
            loading: false,
            search: String::new(),
            sort: Sort::Usage,
            desc: true,
            filter: Filter::All,
            selected: None,
            detail: None,
        }
    }
}

pub fn load(app: &mut App) {
    app.stats.loading = true;
    let days = app.stats.days;
    app.net.spawn(async move {
        let r = net::get::<SiteStats>(&format!("/api/stats?days={days}")).await;
        move |app: &mut App| {
            app.stats.loading = false;
            match r {
                // Ignore answers for a period no longer chosen.
                Ok(s) if s.days == app.stats.days => app.stats.site = Some(s),
                Ok(_) => {}
                Err(e) => app.fail(e),
            }
        }
    });
    if let Some(id) = app.stats.selected {
        load_detail(app, id);
    }
}

fn load_detail(app: &mut App, id: AccountId) {
    app.stats.selected = Some(id);
    let days = app.stats.days;
    app.net.spawn(async move {
        let r = net::get::<AccountDetailStats>(&format!("/api/stats/accounts/{id}?days={days}")).await;
        move |app: &mut App| match r {
            Ok(d) if app.stats.selected == Some(d.account.id) && d.days == app.stats.days => app.stats.detail = Some(d),
            Ok(_) => {}
            Err(e) => {
                app.stats.selected = None;
                app.stats.detail = None;
                app.fail(e);
            }
        }
    });
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let compact = app.compact;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(if compact { 12 } else { 28 }, 18)))
        .show(ui, |ui| {
            header(app, ui);
            ui.add_space(10.0);
            let Some(site) = app.stats.site.clone() else {
                ui.centered_and_justified(|ui| ui.spinner());
                return;
            };
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                tiles(ui, &site);
                ui.add_space(14.0);
                charts(ui, &site, compact);
                ui.add_space(14.0);
                accounts_card(app, ui, &site);
            });
        });
    detail_window(app, &ctx);
}

fn header(app: &mut App, ui: &mut Ui) {
    let compact = app.compact;
    let mut reload = false;
    widgets::screen_header(
        ui,
        compact,
        |ui| {
            ui.heading("Statistics");
        },
        |ui| {
            if ui.add_enabled(!app.stats.loading, egui::Button::new("Refresh")).clicked() {
                reload = true;
            }
            if app.stats.loading {
                ui.spinner();
            }
            for (days, label) in WINDOWS.iter().rev() {
                if ui.add(egui::Button::selectable(app.stats.days == *days, *label)).clicked() && app.stats.days != *days {
                    app.stats.days = *days;
                    app.stats.detail = None;
                    reload = true;
                }
            }
        },
    );
    if reload {
        load(app);
    }
}

// ---- tiles ------------------------------------------------------------------

fn tiles(ui: &mut Ui, s: &SiteStats) {
    let t = &s.totals;
    let pct = |n: u32| if t.accounts > 0 { format!("{:.0} % of accounts", n as f64 * 100.0 / t.accounts as f64) } else { String::new() };
    let period: u64 = s.daily.iter().map(|d| d.minutes).sum();
    let items = [
        ("Accounts", t.accounts.to_string(), format!("+{} in 7 days · +{} in 30", t.new_7, t.new_30)),
        ("Active today", t.active_1.to_string(), pct(t.active_1)),
        ("Active 7 days", t.active_7.to_string(), pct(t.active_7)),
        ("Active 30 days", t.active_30.to_string(), pct(t.active_30)),
        ("Premium", t.premium.to_string(), pct(t.premium)),
        ("Time used", util::fmt_minutes(period), format!("in the last {}", window_label(s.days))),
        ("Teams", t.teams.to_string(), format!("{} admins", t.admins)),
        ("Choreographies", t.choreographies.to_string(), format!("{} with music", t.music_tracks)),
        ("Storage used", util::fmt_bytes(t.bytes_used), String::new()),
    ];
    tile_grid(ui, &items);
    let since = match t.tracking_since {
        Some(d) => format!("Activity (time, logins, edits, exports) is recorded since {}.", util::fmt_date(d)),
        None => "No activity recorded yet: time, logins and edits are counted from now on.".to_owned(),
    };
    ui.add_space(4.0);
    ui.label(RichText::new(since).small().color(theme::TEXT_FAINT));
}

/// Figure tiles in rows of equal width: label, big value, small note.
fn tile_grid(ui: &mut Ui, items: &[(&str, String, String)]) {
    const GAP: f32 = 10.0;
    const MIN_W: f32 = 165.0;
    let avail = ui.available_width();
    let cols = (((avail + GAP) / (MIN_W + GAP)).floor() as usize).clamp(1, items.len().max(1));
    let w = (avail - GAP * (cols as f32 - 1.0)) / cols as f32;
    for row in items.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for (label, value, sub) in row {
                tile(ui, w, label, value, sub);
            }
        });
        ui.add_space(GAP);
    }
}

fn tile(ui: &mut Ui, width: f32, label: &str, value: &str, sub: &str) {
    let margin = 14.0;
    widgets::card().inner_margin(Margin::symmetric(margin as i8, 10)).show(ui, |ui| {
        let inner = width - 2.0 * margin - 2.0;
        ui.set_width(inner);
        ui.vertical(|ui| {
            ui.set_width(inner);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::Label::new(RichText::new(label.to_uppercase()).small().strong().color(theme::TEXT_FAINT)).truncate());
            ui.add(egui::Label::new(RichText::new(value).size(24.0).strong().color(theme::TEXT)).truncate());
            ui.add(
                egui::Label::new(RichText::new(if sub.is_empty() { " " } else { sub }).small().color(theme::TEXT_WEAK))
                    .truncate(),
            )
            .on_hover_text(sub);
        });
    });
}

fn window_label(days: u32) -> &'static str {
    WINDOWS.iter().find(|(d, _)| *d == days).map_or("period", |(_, l)| l)
}

// ---- charts -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Bars,
    Line,
}

struct Series<'a> {
    title: &'a str,
    /// One value per day.
    values: Vec<f64>,
    agg: Agg,
    mark: Mark,
    color: Color32,
    fmt: fn(f64) -> String,
    /// Axis step unit: 60 for minutes (round hours), 1 otherwise.
    unit: f64,
}

fn charts(ui: &mut Ui, s: &SiteStats, compact: bool) {
    let days: Vec<i64> = s.daily.iter().map(|d| d.day).collect();
    let col = |f: fn(&DayStats) -> u64| s.daily.iter().map(|d| f(d) as f64).collect::<Vec<_>>();
    // Accounts that existed at the end of each day.
    let before = s.accounts.iter().filter(|a| util_day(a.created_at) < days[0]).count() as f64;
    let mut total = before;
    let cumulative: Vec<f64> = s
        .daily
        .iter()
        .map(|d| {
            total += d.signups as f64;
            total
        })
        .collect();
    let int = |v: f64| util::fmt_count(v);
    let hours = |v: f64| util::fmt_minutes(v.round() as u64);
    let series = [
        Series { title: "Active accounts per day", values: col(|d| d.active as u64), agg: Agg::Mean, mark: Mark::Bars, color: theme::accent(), fmt: int, unit: 1.0 },
        Series { title: "Accounts in total", values: cumulative, agg: Agg::Last, mark: Mark::Line, color: theme::SELECT, fmt: int, unit: 1.0 },
        Series { title: "New accounts", values: col(|d| d.signups as u64), agg: Agg::Sum, mark: Mark::Bars, color: theme::OK, fmt: int, unit: 1.0 },
        Series { title: "Time used", values: col(|d| d.minutes), agg: Agg::Sum, mark: Mark::Bars, color: theme::accent(), fmt: hours, unit: 60.0 },
        Series { title: "Edits", values: col(|d| d.edits), agg: Agg::Sum, mark: Mark::Bars, color: theme::WARN, fmt: int, unit: 1.0 },
        Series { title: "Logins", values: col(|d| d.logins), agg: Agg::Sum, mark: Mark::Bars, color: theme::SELECT, fmt: int, unit: 1.0 },
        Series { title: "Choreographies created", values: col(|d| d.created), agg: Agg::Sum, mark: Mark::Bars, color: theme::OK, fmt: int, unit: 1.0 },
        Series { title: "Downloads (.arrus + PDF)", values: col(|d| d.exports), agg: Agg::Sum, mark: Mark::Bars, color: theme::WARN, fmt: int, unit: 1.0 },
    ];
    grid(ui, compact, series.len() + 2, |ui, i| {
        if let Some(sr) = series.get(i) {
            chart_card(ui, sr, &days);
        } else if i == series.len() {
            weekday_card(ui, &s.daily);
        } else {
            teams_card(ui, s);
        }
    });
}

/// `n` cards, two per row (one on phones).
fn grid(ui: &mut Ui, compact: bool, n: usize, mut card: impl FnMut(&mut Ui, usize)) {
    let cols = if compact || ui.available_width() < 700.0 { 1 } else { 2 };
    let gap = 12.0;
    let w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    for row in (0..n).collect::<Vec<_>>().chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &i in row {
                ui.vertical(|ui| {
                    ui.set_width(w);
                    card(ui, i);
                });
            }
        });
        ui.add_space(gap);
    }
}

fn util_day(unix: i64) -> i64 {
    unix.div_euclid(86_400)
}

fn chart_card(ui: &mut Ui, s: &Series<'_>, days: &[i64]) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let headline = match s.agg {
            Agg::Sum => (s.fmt)(s.values.iter().sum()),
            Agg::Mean => format!("Ø {}", (s.fmt)(s.values.iter().sum::<f64>() / s.values.len().max(1) as f64)),
            Agg::Last => (s.fmt)(*s.values.last().unwrap_or(&0.0)),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(s.title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(headline).strong().color(s.color));
            });
        });
        ui.add_space(4.0);
        chart(ui, s, days, 140.0);
    });
}

/// A bar or line chart of one value per day, with hover read-outs.
fn chart(ui: &mut Ui, s: &Series<'_>, days: &[i64], height: f32) {
    let size = bucket_size(s.values.len());
    let values = bucketed(&s.values, size, s.agg);
    let starts: Vec<i64> = days.chunks(size).map(|c| c[0]).collect();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect.expand(2.0));
    let font = FontId::proportional(10.0);
    let label_w = 34.0;
    let plot = egui::Rect::from_min_max(pos2(rect.left() + label_w, rect.top() + 4.0), pos2(rect.right(), rect.bottom() - 16.0));
    let max = values.iter().cloned().fold(0.0_f64, f64::max);
    let top = nice_max(max / s.unit) * s.unit;
    // Grid lines at 0, ½ and the top.
    for f in [0.0, 0.5, 1.0] {
        let y = plot.bottom() - plot.height() * f;
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, theme::border()));
        p.text(pos2(plot.left() - 4.0, y), Align2::RIGHT_CENTER, (s.fmt)(top * f as f64), font.clone(), theme::TEXT_FAINT);
    }
    let n = values.len().max(1);
    let step = plot.width() / n as f32;
    let x_of = |i: usize| plot.left() + step * (i as f32 + 0.5);
    let y_of = |v: f64| plot.bottom() - plot.height() * (if top > 0.0 { (v / top) as f32 } else { 0.0 });
    let hovered = resp.hover_pos().filter(|h| plot.x_range().contains(h.x)).map(|h| (((h.x - plot.left()) / step) as usize).min(n - 1));
    match s.mark {
        Mark::Bars => {
            let w = (step * 0.75).clamp(1.0, 28.0);
            for (i, v) in values.iter().enumerate() {
                if *v <= 0.0 {
                    continue;
                }
                let r = egui::Rect::from_min_max(pos2(x_of(i) - w / 2.0, y_of(*v)), pos2(x_of(i) + w / 2.0, plot.bottom()));
                let c = if hovered == Some(i) { s.color } else { s.color.gamma_multiply(0.75) };
                p.rect_filled(r, egui::CornerRadius::same(if w > 6.0 { 2 } else { 0 }), c);
            }
        }
        Mark::Line => {
            let pts: Vec<_> = values.iter().enumerate().map(|(i, v)| pos2(x_of(i), y_of(*v))).collect();
            if pts.len() == 1 {
                p.circle_filled(pts[0], 3.0, s.color);
            }
            p.add(egui::Shape::line(pts, Stroke::new(2.0, s.color)));
        }
    }
    // Dates under the first, middle and last columns.
    let mut marks = vec![0, n / 2, n - 1];
    marks.dedup();
    for (k, &i) in marks.iter().enumerate() {
        let Some(&d) = starts.get(i) else { continue };
        let align = match k {
            0 => Align2::LEFT_TOP,
            _ if i == n - 1 => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        let x = match align {
            Align2::LEFT_TOP => plot.left(),
            Align2::RIGHT_TOP => plot.right(),
            _ => x_of(i),
        };
        p.text(pos2(x, plot.bottom() + 3.0), align, util::fmt_day(d), font.clone(), theme::TEXT_FAINT);
    }
    if let Some(i) = hovered {
        let x = x_of(i);
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, theme::TEXT_FAINT.gamma_multiply(0.5)));
        let when = if size == 1 {
            util::fmt_date(starts[i])
        } else {
            let last = (starts[i] + size as i64 - 1).min(*days.last().unwrap_or(&starts[i]));
            format!("{} – {}", util::fmt_day(starts[i]), util::fmt_date(last))
        };
        let what = match (s.agg, size) {
            (Agg::Mean, 2..) => format!("Ø {} per day", (s.fmt)(values[i])),
            _ => (s.fmt)(values[i]),
        };
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(RichText::new(when).small().color(theme::TEXT_WEAK));
            ui.label(RichText::new(what).strong());
        });
    }
}

fn weekday_card(ui: &mut Ui, daily: &[DayStats]) {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut minutes = [0.0f64; 7];
    for d in daily {
        minutes[util::weekday(d.day)] += d.minutes as f64;
    }
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Time used by weekday").strong());
        ui.add_space(4.0);
        let max = minutes.iter().cloned().fold(0.0, f64::max);
        for (i, name) in NAMES.iter().enumerate() {
            hbar(ui, name, minutes[i], max, util::fmt_minutes(minutes[i] as u64), theme::accent());
        }
    });
}

fn teams_card(ui: &mut Ui, s: &SiteStats) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Largest teams").strong());
        ui.add_space(4.0);
        let max = s.teams.first().map_or(0.0, |t| t.bytes as f64);
        for t in s.teams.iter().take(8) {
            let owner = t.owner.as_deref().map(|o| format!(" · {o}")).unwrap_or_default();
            let r = hbar(ui, &t.name, t.bytes as f64, max, util::fmt_bytes(t.bytes), theme::SELECT);
            r.on_hover_text(format!(
                "{}{owner}\n{} people · {} choreographies · {}",
                t.name,
                t.members,
                t.choreographies,
                util::fmt_bytes(t.bytes)
            ));
        }
        if s.teams.is_empty() {
            ui.label(RichText::new("No teams yet.").color(theme::TEXT_WEAK));
        }
    });
}

/// A labelled horizontal bar: `label  ████░░░  value`.
fn hbar(ui: &mut Ui, label: &str, v: f64, max: f64, value: String, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let p = ui.painter_at(rect);
    let font = FontId::proportional(12.0);
    let label_w = (rect.width() * 0.3).min(120.0);
    let value_w = 70.0;
    let short: String = if label.chars().count() > 16 { label.chars().take(15).chain(['…']).collect() } else { label.to_owned() };
    p.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, short, font.clone(), theme::TEXT_WEAK);
    let track = egui::Rect::from_min_max(
        pos2(rect.left() + label_w, rect.center().y - 5.0),
        pos2(rect.right() - value_w, rect.center().y + 5.0),
    );
    p.rect_filled(track, egui::CornerRadius::same(3), theme::raised());
    if max > 0.0 && v > 0.0 {
        let w = (track.width() * (v / max) as f32).max(2.0);
        p.rect_filled(egui::Rect::from_min_size(track.min, vec2(w, track.height())), egui::CornerRadius::same(3), color);
    }
    p.text(pos2(rect.right(), rect.center().y), Align2::RIGHT_CENTER, value, font, theme::TEXT);
    ui.add_space(2.0);
    resp
}

// ---- accounts ---------------------------------------------------------------

fn sorted<'a>(st: &Stats, site: &'a SiteStats) -> Vec<&'a AccountStats> {
    let q = st.search.trim().to_lowercase();
    let mut v: Vec<&AccountStats> = site
        .accounts
        .iter()
        .filter(|a| st.filter.keeps(a))
        .filter(|a| {
            q.is_empty() || a.nickname.to_lowercase().contains(&q) || a.email.as_deref().is_some_and(|e| e.to_lowercase().contains(&q))
        })
        .collect();
    v.sort_by(|a, b| {
        let o = match st.sort {
            Sort::Usage => a.minutes.cmp(&b.minutes),
            Sort::TotalUsage => a.minutes_total.cmp(&b.minutes_total),
            Sort::LastActive => a.last_active_day.cmp(&b.last_active_day),
            Sort::Edits => a.edits.cmp(&b.edits),
            Sort::Choreographies => a.choreographies.cmp(&b.choreographies),
            Sort::Storage => a.bytes.cmp(&b.bytes),
            Sort::Teams => a.teams.cmp(&b.teams),
            Sort::Joined => a.created_at.cmp(&b.created_at),
            Sort::Name => a.nickname.to_lowercase().cmp(&b.nickname.to_lowercase()),
        };
        // Ties: by name.
        let o = if st.desc { o.reverse() } else { o };
        o.then_with(|| a.nickname.to_lowercase().cmp(&b.nickname.to_lowercase()))
    });
    v
}

fn accounts_card(app: &mut App, ui: &mut Ui, site: &SiteStats) {
    let compact = app.compact;
    let mut open = None;
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section(ui, "Accounts");
        let st = &mut app.stats;
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("Search nickname or email").desired_width(200.0));
            for f in Filter::ALL {
                if ui.add(egui::Button::selectable(st.filter == f, f.label())).clicked() {
                    st.filter = f;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Sort by").color(theme::TEXT_WEAK));
            egui::ComboBox::from_id_salt("stats_sort").selected_text(st.sort.label()).width(180.0).show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.add(egui::Button::selectable(st.sort == s, s.label())).clicked() {
                        st.sort = s;
                        st.desc = s.natural_desc();
                    }
                }
            });
            let arrow = if st.desc { "↓ Descending" } else { "↑ Ascending" };
            if ui.button(arrow).on_hover_text("Reverse the order").clicked() {
                st.desc = !st.desc;
            }
        });
        ui.add_space(6.0);
        let list = sorted(&app.stats, site);
        ui.label(RichText::new(format!("{} of {} accounts · click one for details", list.len(), site.accounts.len())).small().color(theme::TEXT_FAINT));
        ui.add_space(4.0);
        if compact {
            for a in &list {
                if account_row_compact(ui, a, site.today).clicked() {
                    open = Some(a.id);
                }
            }
        } else {
            open = account_table(ui, &mut app.stats, &list, site.today);
        }
        if list.is_empty() {
            ui.label(RichText::new("No account matches.").color(theme::TEXT_WEAK));
        }
    });
    if let Some(id) = open {
        app.stats.detail = None;
        load_detail(app, id);
    }
}

fn last_active(a: &AccountStats, today: i64) -> String {
    match a.last_active_day {
        Some(d) if d == today => "today".into(),
        Some(d) if d == today - 1 => "yesterday".into(),
        Some(d) if today - d < 14 => format!("{} days ago", today - d),
        Some(d) => util::fmt_date(d),
        None => "never".into(),
    }
}

fn plan_badge(ui: &mut Ui, a: &AccountStats) {
    if a.is_admin {
        widgets::badge(ui, "ADMIN", theme::SELECT);
    } else if a.is_premium {
        widgets::badge(ui, "PREMIUM", theme::accent());
    }
}

/// The wide table; returns the account clicked. Headers sort.
fn account_table(ui: &mut Ui, st: &mut Stats, list: &[&AccountStats], today: i64) -> Option<AccountId> {
    let mut open = None;
    let cols: [(&str, Option<Sort>); 8] = [
        ("Account", Some(Sort::Name)),
        ("Last active", Some(Sort::LastActive)),
        ("Active days", None),
        ("Time (period)", Some(Sort::Usage)),
        ("Edits (period)", Some(Sort::Edits)),
        ("Choreos", Some(Sort::Choreographies)),
        ("Storage", Some(Sort::Storage)),
        ("Joined", Some(Sort::Joined)),
    ];
    egui::Grid::new("stats_accounts").num_columns(cols.len()).striped(true).spacing([18.0, 6.0]).show(ui, |ui| {
        for (name, sort) in cols {
            match sort {
                Some(s) => {
                    let on = st.sort == s;
                    let text = if on { format!("{name} {}", if st.desc { "↓" } else { "↑" }) } else { name.to_owned() };
                    let r = ui.add(egui::Label::new(RichText::new(text).small().strong().color(if on { theme::TEXT } else { theme::TEXT_FAINT })).sense(Sense::click()));
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        if on {
                            st.desc = !st.desc;
                        } else {
                            st.sort = s;
                            st.desc = s.natural_desc();
                        }
                    }
                }
                None => {
                    ui.label(RichText::new(name).small().strong().color(theme::TEXT_FAINT));
                }
            }
        }
        ui.end_row();
        for a in list {
            let mut clicked = false;
            ui.horizontal(|ui| {
                let r = ui.add(egui::Label::new(RichText::new(&a.nickname).strong().color(theme::accent())).sense(Sense::click()));
                clicked |= r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                plan_badge(ui, a);
            });
            ui.label(last_active(a, today));
            ui.label(a.active_days.to_string());
            ui.label(util::fmt_minutes(a.minutes));
            ui.label(a.edits.to_string());
            ui.label(a.choreographies.to_string());
            ui.label(util::fmt_bytes(a.bytes));
            ui.label(util::fmt_date(util_day(a.created_at)));
            ui.end_row();
            if clicked {
                open = Some(a.id);
            }
        }
    });
    open
}

fn account_row_compact(ui: &mut Ui, a: &AccountStats, today: i64) -> egui::Response {
    let r = Frame::new()
        .fill(theme::raised())
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&a.nickname).strong());
                plan_badge(ui, a);
            });
            ui.label(
                RichText::new(format!(
                    "{} · {} · {} edits · {} choreos · {}",
                    last_active(a, today),
                    util::fmt_minutes(a.minutes),
                    a.edits,
                    a.choreographies,
                    util::fmt_bytes(a.bytes)
                ))
                .small()
                .color(theme::TEXT_WEAK),
            );
        })
        .response
        .interact(Sense::click());
    ui.add_space(4.0);
    r
}

// ---- one account --------------------------------------------------------------

fn detail_window(app: &mut App, ctx: &egui::Context) {
    if app.stats.selected.is_none() {
        return;
    }
    let compact = app.compact;
    let r = egui::Modal::new(egui::Id::unique("stats_detail")).show(ctx, |ui| {
        widgets::fit_modal(ui, 820.0);
        let Some(d) = app.stats.detail.clone() else {
            ui.set_min_width(widgets::modal_width(ui.ctx(), 820.0));
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading…");
            });
            return ui.button("Close").clicked();
        };
        let mut close = false;
        ui.horizontal(|ui| {
            ui.heading(&d.account.nickname);
            plan_badge(ui, &d.account);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| close = ui.button("Close").clicked());
        });
        let max_h = ui.ctx().content_rect().height() * 0.75;
        ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            detail_body(ui, &d, compact);
        });
        close
    });
    if r.inner || r.should_close() {
        app.stats.selected = None;
        app.stats.detail = None;
    }
}

fn detail_body(ui: &mut Ui, d: &AccountDetailStats, compact: bool) {
    let a = &d.account;
    let email = a.email.as_deref().unwrap_or("no email");
    ui.label(
        RichText::new(format!(
            "{email} · joined {} · last active {}",
            util::fmt_date(util_day(a.created_at)),
            last_active(a, d.today)
        ))
        .color(theme::TEXT_WEAK),
    );
    ui.add_space(8.0);
    let period = window_label(d.days);
    let items = [
        ("Time used", util::fmt_minutes(a.minutes), format!("{period} · {} all time", util::fmt_minutes(a.minutes_total))),
        ("Active days", a.active_days.to_string(), format!("of the last {}", d.days)),
        ("Edits", a.edits.to_string(), format!("{period} · {} all time", a.edits_total)),
        ("Logins", a.logins_total.to_string(), "all time".to_owned()),
        ("Downloads", a.exports_total.to_string(), ".arrus + PDF, all time".to_owned()),
        ("Teams", a.teams.to_string(), format!("owns {}", a.owned_teams)),
        ("Choreographies", a.choreographies.to_string(), "in teams it owns".to_owned()),
        ("Storage", util::fmt_bytes(a.bytes), "of teams it owns".to_owned()),
    ];
    tile_grid(ui, &items);
    ui.add_space(10.0);

    let days: Vec<i64> = d.daily.iter().map(|x| x.day).collect();
    let col = |f: fn(&DayStats) -> u64| d.daily.iter().map(|x| f(x) as f64).collect::<Vec<_>>();
    let int = |v: f64| util::fmt_count(v);
    let hours = |v: f64| util::fmt_minutes(v.round() as u64);
    let series = [
        Series { title: "Time used", values: col(|x| x.minutes), agg: Agg::Sum, mark: Mark::Bars, color: theme::accent(), fmt: hours, unit: 60.0 },
        Series { title: "Edits", values: col(|x| x.edits), agg: Agg::Sum, mark: Mark::Bars, color: theme::WARN, fmt: int, unit: 1.0 },
    ];
    grid(ui, compact, series.len(), |ui, i| chart_card(ui, &series[i], &days));

    widgets::section(ui, "Teams");
    if d.teams.is_empty() {
        ui.label(RichText::new("In no team.").color(theme::TEXT_WEAK));
    }
    for t in &d.teams {
        let role = match t.role {
            TeamRole::Owner => "owner",
            TeamRole::CoOwner => "co-owner",
            TeamRole::Guest => "guest",
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&t.team.name).strong());
            ui.label(
                RichText::new(format!(
                    "{role} · {} people · {} choreographies · {}",
                    t.team.members,
                    t.team.choreographies,
                    util::fmt_bytes(t.team.bytes)
                ))
                .small()
                .color(theme::TEXT_WEAK),
            );
        });
    }
    ui.add_space(8.0);
    widgets::section(ui, "Choreographies in its teams");
    if d.choreographies.is_empty() {
        ui.label(RichText::new("None.").color(theme::TEXT_WEAK));
        return;
    }
    egui::Grid::new("stats_detail_choreos").num_columns(if compact { 2 } else { 5 }).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
        for c in &d.choreographies {
            ui.vertical(|ui| {
                ui.label(RichText::new(&c.name).strong());
                ui.label(RichText::new(&c.team).small().color(theme::TEXT_FAINT));
            });
            let shape = format!("{} formations · {} dancers{}", c.formation_count, c.dancer_count, if c.has_music { " · ♪" } else { "" });
            let changed = format!("changed {} by {}", util::fmt_date(util_day(c.updated_at)), c.updated_by);
            if compact {
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("{shape} · {}", util::fmt_bytes(c.bytes))).small());
                    ui.label(RichText::new(changed).small().color(theme::TEXT_WEAK));
                });
            } else {
                ui.label(RichText::new(shape).small());
                ui.label(RichText::new(util::fmt_bytes(c.bytes)).small());
                ui.label(RichText::new(format!("created {}", util::fmt_date(util_day(c.created_at)))).small().color(theme::TEXT_WEAK));
                ui.label(RichText::new(changed).small().color(theme::TEXT_WEAK));
            }
            ui.end_row();
        }
    });
}
