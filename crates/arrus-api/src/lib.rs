//! The HTTP API between `arrus-server` and `arrus-web`. Both sides compile against
//! these types, so they cannot drift apart.
//!
//! All JSON. Session: an `arus_session` cookie (HttpOnly, SameSite=Lax) set by the
//! HTML login form. Every mutating `/api` request must send the header
//! `X-Arus: 1` (CSRF guard); the server rejects it otherwise with 403.
//!
//! Errors: a non-2xx status with an [`ApiError`] body, except 409 on save, whose
//! body is a [`Conflict`]. 401 = not logged in, 403 = not allowed,
//! 404 = not found or not in your team, 413 = file too big,
//! 507 = storage full ([`ApiError::code`] = `"storage_full"`).
//!
//! | method | path | body | response | who |
//! |---|---|---|---|---|
//! | GET  | `/login?next=` | | HTML login form | anyone |
//! | POST | `/login` | form `username` (nickname or email), `password`, `next` | 303 to `next` or `/` (or back to `/login?error=1`) | anyone |
//! | GET  | `/signup?next=` | | HTML sign-up form | anyone |
//! | POST | `/signup` | form `nickname`, `email`, `password`, `password2`, `next` | 303 to `next` or `/`, logged in; or the form again with an error | anyone |
//! | GET  | `/join/{token}` | | joins the team, 303 to `/#team={id}` (to `/login?next=…` when logged out) | anyone |
//! | POST | `/logout` | | 303 to `/login` | logged in |
//! | GET  | `/healthz` | | `ok` | anyone |
//! | GET  | `/api/version` | | [`Version`] | anyone |
//! | GET  | `/api/me` | | [`Me`] | logged in |
//! | PATCH | `/api/me` | [`UpdateMe`] | [`Me`]; 403 `wrong_password`, 429 `nickname_too_soon` | accounts |
//! | PUT  | `/api/me/prefs` | [`UserPrefs`] | [`UserPrefs`] (cleaned) | logged in |
//! | GET  | `/api/storage` | | [`Storage`] | logged in (per-team breakdown for admin only) |
//! | GET  | `/api/teams` | | `Vec<Team>`: all teams | admin |
//! | POST | `/api/teams` | [`CreateTeam`] | [`Team`]; 403 `limit_teams` | logged in (an account owns what it creates) |
//! | PATCH | `/api/teams/{id}` | [`UpdateTeam`] | [`Team`] | owner, admin |
//! | DELETE | `/api/teams/{id}` | | 204; deletes its choreographies and music | owner, admin |
//! | GET  | `/api/teams/{id}/members` | | `Vec<TeamMember>` | team members, admin |
//! | POST | `/api/teams/{id}/members` | [`AddTeamMember`] | [`TeamMember`]; 404 `no_such_account` | owner, admin |
//! | PATCH | `/api/teams/{id}/members/{account}` | [`UpdateTeamMember`] | [`TeamMember`] | owner, admin |
//! | DELETE | `/api/teams/{id}/members/{account}` | | 204 | owner, admin, or the member (leaving) |
//! | GET  | `/api/teams/{id}/invites` | | `Vec<Invite>` | owner, admin |
//! | POST | `/api/teams/{id}/invites` | [`CreateInvite`] | [`Invite`] | owner, admin |
//! | DELETE | `/api/invites/{token}` | | 204 | owner, admin |
//! | GET  | `/api/accounts` | | `Vec<AccountInfo>` | admin |
//! | POST | `/api/accounts` | [`CreateAccount`] | [`AccountInfo`] | admin |
//! | PATCH | `/api/accounts/{id}` | [`UpdateAccount`] | [`AccountInfo`] | admin |
//! | DELETE | `/api/accounts/{id}` | | 204; also deletes the teams it owns | admin |
//! | GET  | `/api/choreographies?team={id}` | | `Vec<ChoreographySummary>` | team members (team implicit), admin (team required) |
//! | POST | `/api/choreographies` | [`CreateChoreography`] | [`ChoreographySummary`] | writers |
//! | GET  | `/api/choreographies/{id}` | | [`ChoreographyDoc`] | team readers |
//! | PUT  | `/api/choreographies/{id}` | [`SaveChoreography`] | [`Saved`], or 409 [`Conflict`] | writers |
//! | DELETE | `/api/choreographies/{id}` | | 204 (and its music) | writers |
//! | POST | `/api/choreographies/{id}/patch` | [`PatchChoreography`] | [`Saved`] | writers |
//! | GET  | `/api/choreographies/{id}/live?session={s}` | | `text/event-stream` of [`LiveEvent`] JSON | team readers |
//! | POST | `/api/choreographies/{id}/presence` | [`PresenceUpdate`] | 204 | team readers |
//! | GET  | `/api/choreographies/{id}/ws?session={s}` | WebSocket: [`LiveRequest`] JSON in, [`LiveEvent`] JSON out | team readers |
//! | GET  | `/api/choreographies/{id}/export` | | `.arrus` bytes (`Content-Disposition: attachment`) | team readers |
//! | GET  | `/api/choreographies/{id}/pdf` | | PDF, one page per formation (`Content-Disposition: attachment`) | team readers |
//! | POST | `/api/teams/{id}/import` | raw `.arrus` / `.aubkp` bytes, header `X-Filename` | [`ImportResult`] | writers of that team |
//! | GET  | `/api/choreographies/{id}/music` | | audio bytes with its content type, HTTP Range supported | team readers |
//! | PUT  | `/api/choreographies/{id}/music` | raw audio bytes, `Content-Type` audio/*, header `X-Filename` | [`ChoreographySummary`] | writers |
//! | DELETE | `/api/choreographies/{id}/music` | | [`ChoreographySummary`] | writers |
//! | GET  | `/api/stats?days={n}` | | [`SiteStats`] (the last `n` days, 1–730, default 30) | admin |
//! | GET  | `/api/stats/accounts/{id}?days={n}` | | [`AccountDetailStats`] | admin |
//! | GET  | `/api/settings/alerts` | | [`AlertSettings`] | admin |
//! | PUT  | `/api/settings/alerts` | [`UpdateAlertSettings`] | [`AlertSettings`]; 400 with code `invalid_webhook` / `invalid_mention` | admin |
//! | POST | `/api/settings/alerts/test` | | 204 once Discord accepted it; 400 `no_webhook`; 502 with Discord's answer | admin |
//!
//! "Writers" = the admin, or the owner or a co-owner of the choreography's team.
//! "Team readers" = the admin, or anyone in the choreography's team.
//! An account in exactly one team may leave the team out (`team_id` / `?team=`).
//! Creating a choreography in a team owned by a free account fails with 403
//! `limit_choreographies` past [`FREE_CHOREOGRAPHIES`].

pub mod patch;

use arrus_core::Choreography;
use serde::{Deserialize, Serialize};

pub use patch::PatchOp;

/// Header every mutating request must carry, with value `1`.
pub const CSRF_HEADER: &str = "X-Arus";
/// Header carrying an uploaded file's original name.
pub const FILENAME_HEADER: &str = "X-Filename";
/// Nickname of the master admin (password from the server's environment).
pub const ADMIN_USERNAME: &str = "admin";
/// Largest music upload accepted.
pub const MAX_MUSIC_BYTES: u64 = 100 * 1024 * 1024;
/// Largest `.arrus`/`.aubkp` import accepted.
pub const MAX_IMPORT_BYTES: u64 = 200 * 1024 * 1024;

pub type TeamId = i64;
pub type AccountId = i64;
pub type ChoreographyId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The master admin, or an account with admin rights: sees and manages everything.
    Admin,
    Member,
}

/// What an account may do in a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    /// Views the choreographies.
    Guest,
    /// Creates, edits and deletes choreographies with the owner.
    CoOwner,
    /// Created the team: also manages who is in it, renames and deletes it.
    Owner,
}

impl TeamRole {
    pub fn can_write(self) -> bool {
        self != TeamRole::Guest
    }
    pub fn as_str(self) -> &'static str {
        match self {
            TeamRole::Guest => "guest",
            TeamRole::CoOwner => "co_owner",
            TeamRole::Owner => "owner",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "guest" => Some(TeamRole::Guest),
            "co_owner" => Some(TeamRole::CoOwner),
            "owner" => Some(TeamRole::Owner),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TeamRole::Guest => "Guest",
            TeamRole::CoOwner => "Co-owner",
            TeamRole::Owner => "Owner",
        }
    }
}

/// Teams a free account may create.
pub const FREE_TEAMS: u32 = 1;
/// Choreographies a team owned by a free account may hold.
pub const FREE_CHOREOGRAPHIES: u32 = 2;

/// Who is logged in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    /// The nickname (`admin` for the master admin).
    pub username: String,
    pub role: Role,
    /// `None` for the master admin.
    pub account_id: Option<AccountId>,
    pub email: Option<String>,
    pub premium: bool,
    /// The teams this account is in.
    pub teams: Vec<MyTeam>,
    /// May create another team (free accounts: [`FREE_TEAMS`]).
    pub can_create_team: bool,
    /// Unix seconds from when the nickname may be changed again (`None`: now).
    #[serde(default)]
    pub nickname_change_from: Option<i64>,
    /// This person's own settings (colours, animation).
    #[serde(default)]
    pub prefs: UserPrefs,
}

/// Seconds between two nickname changes of an account (30 days).
pub const NICKNAME_CHANGE_INTERVAL: i64 = 30 * 24 * 3600;

/// Personal settings, kept on the server so they follow the account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPrefs {
    /// UI background colour (sRGB); `None`: the default. Kept dark.
    #[serde(default)]
    pub background: Option<[u8; 3]>,
    /// Accent colour (sRGB); `None`: the default violet.
    #[serde(default)]
    pub accent: Option<[u8; 3]>,
    /// How long switching between formations by hand animates, in seconds
    /// (0: no animation), up to [`MAX_GLIDE_SECS`].
    #[serde(default = "default_glide_secs")]
    pub glide_secs: f64,
    /// Warn when a dancer moves farther than this share of the stage (its
    /// longer side) in one transition; 0..=1, 0: no warning.
    #[serde(default = "default_long_move_warn")]
    pub long_move_warn: f64,
}

pub const DEFAULT_GLIDE_SECS: f64 = 0.45;
pub const MAX_GLIDE_SECS: f64 = 2.0;

fn default_glide_secs() -> f64 {
    DEFAULT_GLIDE_SECS
}

pub const DEFAULT_LONG_MOVE_WARN: f64 = 0.0;

fn default_long_move_warn() -> f64 {
    DEFAULT_LONG_MOVE_WARN
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self { background: None, accent: None, glide_secs: DEFAULT_GLIDE_SECS, long_move_warn: DEFAULT_LONG_MOVE_WARN }
    }
}

impl UserPrefs {
    /// Within range (a NaN or out-of-range value is clamped).
    pub fn cleaned(mut self) -> Self {
        self.glide_secs = if self.glide_secs.is_finite() { self.glide_secs.clamp(0.0, MAX_GLIDE_SECS) } else { DEFAULT_GLIDE_SECS };
        self.long_move_warn =
            if self.long_move_warn.is_finite() { self.long_move_warn.clamp(0.0, 1.0) } else { DEFAULT_LONG_MOVE_WARN };
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MyTeam {
    pub id: TeamId,
    pub name: String,
    pub role: TeamRole,
    /// The owner's nickname (`None`: a team made by the admin).
    pub owner: Option<String>,
    pub choreography_count: u32,
    /// `None`: unlimited (premium or admin-made team).
    pub choreography_limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    pub owner_id: Option<AccountId>,
    pub owner: Option<String>,
    pub member_count: u32,
    pub choreography_count: u32,
    pub choreography_limit: Option<u32>,
    /// Choreographies + music of this team.
    pub bytes_used: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateTeam {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateTeam {
    pub name: String,
}

/// Someone in a team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamMember {
    pub account_id: AccountId,
    pub nickname: String,
    pub role: TeamRole,
}

/// Add an existing account to a team by its nickname.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddTeamMember {
    pub nickname: String,
    /// `Guest` or `CoOwner`.
    pub role: TeamRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateTeamMember {
    /// `Guest` or `CoOwner`.
    pub role: TeamRole,
}

/// A share link: whoever opens `/join/{token}` (logged in) joins the team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub token: String,
    pub team_id: TeamId,
    pub role: TeamRole,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateInvite {
    /// `Guest` or `CoOwner`.
    pub role: TeamRole,
}

/// An account as the admin sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: AccountId,
    pub nickname: String,
    pub email: Option<String>,
    /// The password, while it is one the server generated or the admin set.
    /// `None` once the person chose their own (stored hashed only).
    pub password: Option<String>,
    pub is_admin: bool,
    pub is_premium: bool,
    pub created_at: i64,
    pub teams: Vec<AccountTeam>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountTeam {
    pub team_id: TeamId,
    pub name: String,
    pub role: TeamRole,
    pub choreographies: Vec<ChoreographyRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoreographyRef {
    pub id: ChoreographyId,
    pub name: String,
}

/// The admin creates an account.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CreateAccount {
    pub nickname: String,
    #[serde(default)]
    pub email: Option<String>,
    /// `None`: generate one (`tiger-maple-42`).
    #[serde(default)]
    pub password: Option<String>,
}

/// The admin changes an account; fields left `None` are unchanged.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UpdateAccount {
    #[serde(default)]
    pub nickname: Option<String>,
    /// `""` removes the email.
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
    #[serde(default)]
    pub is_premium: Option<bool>,
    /// Set this password (the admin can see it; ends their sessions).
    #[serde(default)]
    pub password: Option<String>,
    /// Replace the password with a generated one (ends their sessions).
    #[serde(default)]
    pub regenerate_password: bool,
}

/// An account changes its own details. Needs the current password.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UpdateMe {
    pub current_password: String,
    /// At most once per [`NICKNAME_CHANGE_INTERVAL`] (429 `nickname_too_soon`).
    #[serde(default)]
    pub nickname: Option<String>,
    /// `""` removes the email.
    #[serde(default)]
    pub email: Option<String>,
    /// At least [`MIN_PASSWORD_LEN`] characters; ends the account's other sessions.
    #[serde(default)]
    pub new_password: Option<String>,
}

/// Shortest password a person may choose.
pub const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoreographySummary {
    pub id: ChoreographyId,
    pub team_id: TeamId,
    pub name: String,
    /// Increments on every save; used to detect concurrent edits.
    pub revision: i64,
    /// Unix seconds.
    pub updated_at: i64,
    pub updated_by: String,
    /// Size of the stored choreography JSON.
    pub size_bytes: u64,
    pub formation_count: u32,
    pub dancer_count: u32,
    pub music: Option<MusicInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicInfo {
    pub filename: String,
    pub content_type: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoreographyDoc {
    pub summary: ChoreographySummary,
    pub data: Choreography,
    /// The requester may change it.
    #[serde(default)]
    pub can_write: bool,
}

/// Create a choreography. The server gives it a fresh UUID (`data.id`) and sets
/// its timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateChoreography {
    /// May be left out by an account in exactly one team.
    #[serde(default)]
    pub team_id: Option<TeamId>,
    pub data: Choreography,
}

/// Save over an existing choreography. Rejected with 409 [`Conflict`] if someone
/// saved since `base_revision`; the client then offers "save as a new
/// choreography" (a [`CreateChoreography`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveChoreography {
    pub base_revision: i64,
    pub data: Choreography,
}

/// Live co-editing: a batch of changes made by one editor session, applied to
/// the latest version (see [`patch`]). Never a conflict: concurrent changes
/// merge field by field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchChoreography {
    /// The sending editor session (its own changes come back tagged with it).
    pub session: String,
    /// Numbers the session's batches; echoed in [`LiveEvent::Patch`].
    pub batch: u64,
    pub ops: Vec<PatchOp>,
}

/// Someone with the choreography open, and where they are in it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Viewer {
    pub session: String,
    pub username: String,
    pub can_write: bool,
    /// The formation they are on (index), once they said so.
    #[serde(default)]
    pub formation: Option<usize>,
    /// The dancers (indexes) they have selected.
    #[serde(default)]
    pub selected: Vec<usize>,
    /// They are dragging the selected dancers right now.
    #[serde(default)]
    pub dragging: bool,
}

/// Where an editor session is: sent when it changes (a few times a second at
/// most); everyone gets the new [`LiveEvent::Presence`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub session: String,
    pub formation: Option<usize>,
    #[serde(default)]
    pub selected: Vec<usize>,
    #[serde(default)]
    pub dragging: bool,
}

/// Most selected dancers reported in a [`PresenceUpdate`].
pub const MAX_PRESENCE_SELECTED: usize = 500;

/// Longest editor session id accepted.
pub const MAX_SESSION_LEN: usize = 64;

/// One message of the live stream of a choreography.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    /// First message, and again after a whole-document save: the current
    /// revision. A client that knows another revision reloads the document.
    Hello { revision: i64 },
    /// Who has the choreography open (sent whenever that changes).
    Presence { viewers: Vec<Viewer> },
    /// A batch of changes, now `revision`. Revisions come in order without
    /// gaps; a gap means messages were missed.
    Patch { revision: i64, session: String, batch: u64, by: String, ops: Vec<PatchOp> },
    /// The music changed (the document did not).
    Summary { summary: ChoreographySummary },
    /// The choreography was deleted.
    Deleted,
    /// WebSocket only: our batch `batch` is stored (as `summary.revision`).
    Saved { batch: u64, summary: ChoreographySummary },
    /// WebSocket only: our batch `batch` was refused, with the HTTP status
    /// and error the patch endpoint would have answered.
    Refused { batch: u64, status: u16, error: ApiError },
    /// Dancers someone is dragging right now, where they are this instant.
    /// Not stored: the change itself follows as a [`LiveEvent::Patch`].
    Moving { session: String, formation: usize, dancers: Vec<MovingDancer> },
}

/// One dancer of a [`LiveEvent::Moving`] / [`LiveRequest::Moving`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MovingDancer {
    pub index: usize,
    pub x: f64,
    pub y: f64,
}

/// Most dancers in one [`LiveRequest::Moving`].
pub const MAX_MOVING: usize = 1000;

/// What an editor sends over the live WebSocket. The session is the one the
/// socket was opened with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveRequest {
    /// Same as `POST …/patch`; answered with [`LiveEvent::Saved`] or [`LiveEvent::Refused`].
    Patch { batch: u64, ops: Vec<PatchOp> },
    /// Same as `POST …/presence`.
    Presence {
        formation: Option<usize>,
        #[serde(default)]
        selected: Vec<usize>,
        #[serde(default)]
        dragging: bool,
    },
    /// Dancers being dragged (a few times a second while dragging), passed on as is.
    Moving { formation: usize, dancers: Vec<MovingDancer> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub summary: ChoreographySummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub current_revision: i64,
    pub updated_by: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportResult {
    pub created: Vec<ChoreographySummary>,
    /// Human-readable notes, e.g. a backup's deleted choreographies that were skipped.
    pub notes: Vec<String>,
}

/// Which web client the server hands out. It changes with every deploy that
/// changes the client, so open pages (and the apps) can offer to reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Short hash of the client's `index.html` (which names its hashed
    /// files); `dev` when the client is not built.
    pub client: String,
}

/// Storage use against the app-wide limit (5 GiB). Counts choreography JSON and
/// music files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Storage {
    pub used: u64,
    pub limit: u64,
    /// Admin only; empty for members.
    pub teams: Vec<TeamUsage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamUsage {
    pub team_id: TeamId,
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    /// Machine-readable reason when useful: `storage_full`, `username_taken`,
    /// `email_taken`, `name_taken`, `no_such_account`, `limit_teams`,
    /// `limit_choreographies`, `wrong_password`, `not_arrus`, `too_big`,
    /// `invalid_webhook`, `invalid_mention`, `no_webhook`, `nickname_too_soon`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Where storage alerts are posted (admin only). The webhook URL is a secret and is
/// never sent back whole: only a hint to recognise it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AlertSettings {
    pub webhook_set: bool,
    /// E.g. `discord.com/api/webhooks/1234…/…wxyz`.
    pub webhook_hint: Option<String>,
    /// Who alerts tag: `@here`, `@everyone`, `<@user id>` or `<@&role id>`; empty for nobody.
    pub mention: String,
}

/// Change the alert settings; fields left `None` are unchanged.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UpdateAlertSettings {
    /// A Discord webhook URL (`https://discord.com/api/webhooks/…`), or `""` to stop alerts.
    #[serde(default)]
    pub webhook: Option<String>,
    /// `@here`, `@everyone`, `user:<id>`, `role:<id>`, `<@id>`, `<@&id>`, or `""` for nobody.
    #[serde(default)]
    pub mention: Option<String>,
}

// ---- statistics (admin) ------------------------------------------------------

/// Days the statistics can look back (activity older than that is deleted).
pub const STATS_MAX_DAYS: u32 = 730;

/// The whole site over the last `days` days. Days are UTC day numbers
/// (Unix seconds / 86 400).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SiteStats {
    pub today: i64,
    pub days: u32,
    pub totals: StatTotals,
    /// One entry per day, oldest first, ending today.
    pub daily: Vec<DayStats>,
    pub accounts: Vec<AccountStats>,
    /// Every team, largest first.
    pub teams: Vec<TeamStats>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StatTotals {
    pub accounts: u32,
    pub premium: u32,
    pub admins: u32,
    pub teams: u32,
    pub choreographies: u32,
    pub music_tracks: u32,
    /// Choreographies + music.
    pub bytes_used: u64,
    /// Accounts active today, in the last 7 and the last 30 days.
    pub active_1: u32,
    pub active_7: u32,
    pub active_30: u32,
    /// Accounts created in the last 7 and 30 days.
    pub new_7: u32,
    pub new_30: u32,
    /// The first day with recorded activity (`None`: nothing recorded yet).
    pub tracking_since: Option<i64>,
}

/// What happened on one day (on the whole site, or for one account).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DayStats {
    pub day: i64,
    /// Accounts created.
    pub signups: u32,
    /// Accounts that used Eightcount (one account: 0 or 1).
    pub active: u32,
    /// Minutes in Eightcount (counted in 5-minute steps).
    pub minutes: u64,
    pub logins: u64,
    /// Saved changes to choreographies.
    pub edits: u64,
    /// Choreographies created or imported.
    pub created: u64,
    /// `.arrus` and PDF downloads.
    pub exports: u64,
    /// Music uploads.
    pub uploads: u64,
}

/// One account's figures; "window" ones cover the `days` asked for.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountStats {
    pub id: AccountId,
    pub nickname: String,
    pub email: Option<String>,
    pub is_admin: bool,
    pub is_premium: bool,
    pub created_at: i64,
    pub last_active_day: Option<i64>,
    pub active_days: u32,
    pub minutes: u64,
    pub minutes_total: u64,
    pub edits: u64,
    pub edits_total: u64,
    pub logins_total: u64,
    pub exports_total: u64,
    /// Teams it is in, and owns.
    pub teams: u32,
    pub owned_teams: u32,
    /// Choreographies and bytes (choreographies + music) of the teams it owns.
    pub choreographies: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TeamStats {
    pub id: TeamId,
    pub name: String,
    pub owner: Option<String>,
    pub members: u32,
    pub choreographies: u32,
    pub bytes: u64,
}

/// One account in detail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountDetailStats {
    pub today: i64,
    pub days: u32,
    pub account: AccountStats,
    /// One entry per day, oldest first, ending today.
    pub daily: Vec<DayStats>,
    /// Its teams, with its role in each.
    pub teams: Vec<AccountTeamStats>,
    /// The choreographies of its teams, last changed first.
    pub choreographies: Vec<StatChoreography>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountTeamStats {
    pub team: TeamStats,
    pub role: TeamRole,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StatChoreography {
    pub id: ChoreographyId,
    pub team: String,
    pub name: String,
    pub formation_count: u32,
    pub dancer_count: u32,
    /// Choreography + music.
    pub bytes: u64,
    pub has_music: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub updated_by: String,
}
