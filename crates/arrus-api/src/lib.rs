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
//! | GET  | `/login` | | HTML login form | anyone |
//! | POST | `/login` | form `username`, `password` | 303 to `/` (or back to `/login?error=1`) | anyone |
//! | POST | `/logout` | | 303 to `/login` | logged in |
//! | GET  | `/healthz` | | `ok` | anyone |
//! | GET  | `/api/me` | | [`Me`] | logged in |
//! | GET  | `/api/storage` | | [`Storage`] | logged in (per-team breakdown for admin only) |
//! | GET  | `/api/teams` | | `Vec<Team>` | admin |
//! | POST | `/api/teams` | [`CreateTeam`] | [`Team`] | admin |
//! | PATCH | `/api/teams/{id}` | [`UpdateTeam`] | [`Team`] | admin |
//! | DELETE | `/api/teams/{id}` | | 204; deletes its members, choreographies and music | admin |
//! | GET  | `/api/teams/{id}/members` | | `Vec<Member>` | admin |
//! | POST | `/api/teams/{id}/members` | [`CreateMember`] | [`Member`] (with generated password) | admin |
//! | PATCH | `/api/members/{id}` | [`UpdateMember`] | [`Member`] | admin |
//! | DELETE | `/api/members/{id}` | | 204; ends their sessions | admin |
//! | GET  | `/api/choreographies?team={id}` | | `Vec<ChoreographySummary>` | team members (team implicit), admin (team required) |
//! | POST | `/api/choreographies` | [`CreateChoreography`] | [`ChoreographySummary`] | writers |
//! | GET  | `/api/choreographies/{id}` | | [`ChoreographyDoc`] | team readers |
//! | PUT  | `/api/choreographies/{id}` | [`SaveChoreography`] | [`Saved`], or 409 [`Conflict`] | writers |
//! | DELETE | `/api/choreographies/{id}` | | 204 (and its music) | writers |
//! | GET  | `/api/choreographies/{id}/export` | | `.arrus` bytes (`Content-Disposition: attachment`) | team readers |
//! | POST | `/api/teams/{id}/import` | raw `.arrus` / `.aubkp` bytes, header `X-Filename` | [`ImportResult`] | writers of that team |
//! | GET  | `/api/choreographies/{id}/music` | | audio bytes with its content type, HTTP Range supported | team readers |
//! | PUT  | `/api/choreographies/{id}/music` | raw audio bytes, `Content-Type` audio/*, header `X-Filename` | [`ChoreographySummary`] | writers |
//! | DELETE | `/api/choreographies/{id}/music` | | [`ChoreographySummary`] | writers |
//! | GET  | `/api/settings/alerts` | | [`AlertSettings`] | admin |
//! | PUT  | `/api/settings/alerts` | [`UpdateAlertSettings`] | [`AlertSettings`]; 400 with code `invalid_webhook` / `invalid_mention` | admin |
//! | POST | `/api/settings/alerts/test` | | 204 once Discord accepted it; 400 `no_webhook`; 502 with Discord's answer | admin |
//!
//! "Writers" = the admin, or a member of the choreography's team with `can_write`.
//! "Team readers" = the admin, or any member of the choreography's team.
//! A member's own team is implicit; the admin names the team (`team_id` / `?team=`).

use arrus_core::Choreography;
use serde::{Deserialize, Serialize};

/// Header every mutating request must carry, with value `1`.
pub const CSRF_HEADER: &str = "X-Arus";
/// Header carrying an uploaded file's original name.
pub const FILENAME_HEADER: &str = "X-Filename";
/// Username of the master admin (password from the server's environment).
pub const ADMIN_USERNAME: &str = "admin";
/// Largest music upload accepted.
pub const MAX_MUSIC_BYTES: u64 = 100 * 1024 * 1024;
/// Largest `.arrus`/`.aubkp` import accepted.
pub const MAX_IMPORT_BYTES: u64 = 200 * 1024 * 1024;

pub type TeamId = i64;
pub type MemberId = i64;
pub type ChoreographyId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
}

/// Who is logged in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub username: String,
    pub role: Role,
    /// `None` for the admin.
    pub team: Option<TeamRef>,
    /// Always true for the admin.
    pub can_write: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamRef {
    pub id: TeamId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    pub member_count: u32,
    pub choreography_count: u32,
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

/// A team member's login. Passwords are generated by the server (easy words like
/// `tiger-maple-42`), cannot be changed by the member, and are visible to the admin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub id: MemberId,
    pub team_id: TeamId,
    pub username: String,
    pub password: String,
    pub can_write: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateMember {
    /// Unique across the whole app (case-insensitive); not `admin`.
    pub username: String,
    pub can_write: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UpdateMember {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub can_write: Option<bool>,
    /// Replace the password with a newly generated one (ends their sessions).
    #[serde(default)]
    pub regenerate_password: bool,
}

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
}

/// Create a choreography. The server gives it a fresh UUID (`data.id`) and sets
/// its timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateChoreography {
    /// Required for the admin; ignored for members (their own team).
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
    /// `name_taken`, `not_arrus`, `too_big`, `invalid_webhook`, `invalid_mention`,
    /// `no_webhook`.
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
