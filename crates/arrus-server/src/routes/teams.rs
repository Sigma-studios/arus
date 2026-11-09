//! Teams, who is in them (and as what), and their share links.

use arrus_api::{
    AddTeamMember, CreateInvite, CreateTeam, FREE_TEAMS, Invite, Team, TeamMember, TeamRole, UpdateTeam, UpdateTeamMember,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rand::RngExt;
use rusqlite::{Connection, OptionalExtension, params};

use super::ApiJson;
use crate::auth::{Auth, User};
use crate::error::{Error, Result};
use crate::{AppState, db, storage};

const TEAM_SELECT: &str = "SELECT t.id, t.name, t.owner_id, a.nickname, \
     (SELECT COUNT(*) FROM team_members m WHERE m.team_id = t.id), \
     (SELECT COUNT(*) FROM choreographies c WHERE c.team_id = t.id), \
     (SELECT COALESCE(SUM(c.size_bytes + COALESCE(c.music_bytes, 0)), 0) FROM choreographies c WHERE c.team_id = t.id), \
     a.is_premium, a.is_admin \
     FROM teams t LEFT JOIN accounts a ON a.id = t.owner_id";

fn team_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Team> {
    let owner_id: Option<i64> = r.get(2)?;
    let premium: Option<bool> = r.get(7)?;
    let admin: Option<bool> = r.get(8)?;
    let limited = owner_id.is_some() && premium == Some(false) && admin == Some(false);
    Ok(Team {
        id: r.get(0)?,
        name: r.get(1)?,
        owner_id,
        owner: r.get(3)?,
        member_count: r.get::<_, i64>(4)? as u32,
        choreography_count: r.get::<_, i64>(5)? as u32,
        choreography_limit: limited.then_some(arrus_api::FREE_CHOREOGRAPHIES),
        bytes_used: r.get::<_, i64>(6)? as u64,
    })
}

pub fn load_team(conn: &Connection, id: i64) -> Result<Team> {
    conn.query_row(&format!("{TEAM_SELECT} WHERE t.id = ?1"), [id], team_from_row)
        .optional()?
        .ok_or_else(Error::not_found)
}

/// Team names are unique per owner (and among the admin's teams).
fn clean_team_name(conn: &Connection, name: &str, owner: Option<i64>, except: Option<i64>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(Error::bad_request("team name must be 1 to 80 characters"));
    }
    let taken: Option<i64> = conn
        .query_row(
            "SELECT id FROM teams WHERE name = ?1 COLLATE NOCASE AND owner_id IS ?2 AND id IS NOT ?3",
            params![name, owner, except],
            |r| r.get(0),
        )
        .optional()?;
    if taken.is_some() {
        return Err(Error::with_code(StatusCode::CONFLICT, "name_taken", "you already have a team with that name"));
    }
    Ok(name.to_string())
}

fn owner_of(conn: &Connection, team_id: i64) -> Result<Option<i64>> {
    conn.query_row("SELECT owner_id FROM teams WHERE id = ?1", [team_id], |r| r.get(0))
        .optional()?
        .ok_or_else(Error::not_found)
}

pub async fn list(State(state): State<AppState>, auth: Auth) -> Result<Json<Vec<Team>>> {
    auth.user.require_admin()?;
    state
        .db(|_, conn| {
            let mut stmt = conn.prepare(&format!("{TEAM_SELECT} ORDER BY t.name COLLATE NOCASE, t.id"))?;
            let teams = stmt.query_map([], team_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Json(teams))
        })
        .await
}

/// The master admin makes teams without an owner (no limits); an account owns
/// the teams it makes.
pub async fn create(State(state): State<AppState>, auth: Auth, ApiJson(body): ApiJson<CreateTeam>) -> Result<Json<Team>> {
    let user = auth.user;
    state
        .db(move |_, conn| {
            let owner = user.account_id();
            if let User::Account(a) = &user
                && !a.is_admin
                && !a.is_premium
            {
                let owned: i64 = conn.query_row("SELECT COUNT(*) FROM teams WHERE owner_id = ?1", [a.id], |r| r.get(0))?;
                if owned as u32 >= FREE_TEAMS {
                    return Err(Error::with_code(
                        StatusCode::FORBIDDEN,
                        "limit_teams",
                        format!("a free account can create {FREE_TEAMS} team (premium: unlimited)"),
                    ));
                }
            }
            let name = clean_team_name(conn, &body.name, owner, None)?;
            let tx = conn.transaction()?;
            let now = db::now();
            tx.execute("INSERT INTO teams (name, created_at, owner_id) VALUES (?1, ?2, ?3)", params![name, now, owner])?;
            let id = tx.last_insert_rowid();
            if let Some(owner) = owner {
                tx.execute(
                    "INSERT INTO team_members (team_id, account_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![id, owner, TeamRole::Owner.as_str(), now],
                )?;
            }
            tx.commit()?;
            Ok(Json(load_team(conn, id)?))
        })
        .await
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<UpdateTeam>,
) -> Result<Json<Team>> {
    auth.user.check_manage(id)?;
    state
        .db(move |_, conn| {
            let owner = owner_of(conn, id)?;
            let name = clean_team_name(conn, &body.name, owner, Some(id))?;
            conn.execute("UPDATE teams SET name = ?1 WHERE id = ?2", params![name, id])?;
            Ok(Json(load_team(conn, id)?))
        })
        .await
}

/// Delete teams with their choreographies, music files and memberships.
pub fn delete_teams(s: &AppState, conn: &mut Connection, ids: &[i64]) -> Result<()> {
    let tx = conn.transaction()?;
    let mut files: Vec<String> = Vec::new();
    for id in ids {
        let mut stmt = tx.prepare("SELECT music_file FROM choreographies WHERE team_id = ?1 AND music_file IS NOT NULL")?;
        files.extend(stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
        drop(stmt);
        // Memberships, invites and choreographies cascade.
        tx.execute("DELETE FROM teams WHERE id = ?1", [id])?;
    }
    tx.commit()?;
    storage::remove_music_files(s, files);
    storage::after_change(s, conn)?;
    Ok(())
}

pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<StatusCode> {
    auth.user.check_manage(id)?;
    state
        .db(move |s, conn| {
            if !db::team_exists(conn, id)? {
                return Err(Error::not_found());
            }
            delete_teams(s, conn, &[id])?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

fn member_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TeamMember> {
    let role: String = r.get(2)?;
    Ok(TeamMember { account_id: r.get(0)?, nickname: r.get(1)?, role: TeamRole::parse(&role).unwrap_or(TeamRole::Guest) })
}

fn load_member(conn: &Connection, team_id: i64, account_id: i64) -> Result<TeamMember> {
    conn.query_row(
        "SELECT a.id, a.nickname, m.role FROM team_members m JOIN accounts a ON a.id = m.account_id \
         WHERE m.team_id = ?1 AND m.account_id = ?2",
        params![team_id, account_id],
        member_from_row,
    )
    .optional()?
    .ok_or_else(Error::not_found)
}

pub async fn list_members(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Json<Vec<TeamMember>>> {
    auth.user.check_read(id)?;
    state
        .db(move |_, conn| {
            if !db::team_exists(conn, id)? {
                return Err(Error::not_found());
            }
            let mut stmt = conn.prepare(
                "SELECT a.id, a.nickname, m.role FROM team_members m JOIN accounts a ON a.id = m.account_id \
                 WHERE m.team_id = ?1 ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'co_owner' THEN 1 ELSE 2 END, \
                 a.nickname_lower",
            )?;
            let members = stmt.query_map([id], member_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Json(members))
        })
        .await
}

/// Owners are made by creating a team, never given.
fn grantable(role: TeamRole) -> Result<()> {
    if role == TeamRole::Owner {
        return Err(Error::bad_request("a team has one owner: choose co-owner or guest"));
    }
    Ok(())
}

pub async fn add_member(
    State(state): State<AppState>,
    auth: Auth,
    Path(team_id): Path<i64>,
    ApiJson(body): ApiJson<AddTeamMember>,
) -> Result<Json<TeamMember>> {
    auth.user.check_manage(team_id)?;
    grantable(body.role)?;
    state
        .db(move |_, conn| {
            if !db::team_exists(conn, team_id)? {
                return Err(Error::not_found());
            }
            let account: Option<i64> = conn
                .query_row("SELECT id FROM accounts WHERE nickname_lower = ?1", [body.nickname.trim().to_lowercase()], |r| {
                    r.get(0)
                })
                .optional()?;
            let Some(account) = account else {
                return Err(Error::with_code(StatusCode::NOT_FOUND, "no_such_account", "no account has that nickname"));
            };
            let existing: Option<String> = conn
                .query_row(
                    "SELECT role FROM team_members WHERE team_id = ?1 AND account_id = ?2",
                    params![team_id, account],
                    |r| r.get(0),
                )
                .optional()?;
            match existing.as_deref().and_then(TeamRole::parse) {
                Some(TeamRole::Owner) => {}
                Some(_) => {
                    conn.execute(
                        "UPDATE team_members SET role = ?1 WHERE team_id = ?2 AND account_id = ?3",
                        params![body.role.as_str(), team_id, account],
                    )?;
                }
                None => {
                    conn.execute(
                        "INSERT INTO team_members (team_id, account_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                        params![team_id, account, body.role.as_str(), db::now()],
                    )?;
                }
            }
            Ok(Json(load_member(conn, team_id, account)?))
        })
        .await
}

pub async fn update_member(
    State(state): State<AppState>,
    auth: Auth,
    Path((team_id, account_id)): Path<(i64, i64)>,
    ApiJson(body): ApiJson<UpdateTeamMember>,
) -> Result<Json<TeamMember>> {
    auth.user.check_manage(team_id)?;
    grantable(body.role)?;
    state
        .db(move |_, conn| {
            let m = load_member(conn, team_id, account_id)?;
            if m.role == TeamRole::Owner {
                return Err(Error::bad_request("the owner's role cannot be changed"));
            }
            conn.execute(
                "UPDATE team_members SET role = ?1 WHERE team_id = ?2 AND account_id = ?3",
                params![body.role.as_str(), team_id, account_id],
            )?;
            Ok(Json(load_member(conn, team_id, account_id)?))
        })
        .await
}

/// The owner or the admin removes someone; anyone but the owner may leave.
pub async fn remove_member(
    State(state): State<AppState>,
    auth: Auth,
    Path((team_id, account_id)): Path<(i64, i64)>,
) -> Result<StatusCode> {
    if auth.user.account_id() != Some(account_id) {
        auth.user.check_manage(team_id)?;
    }
    state
        .db(move |_, conn| {
            let m = load_member(conn, team_id, account_id)?;
            if m.role == TeamRole::Owner {
                return Err(Error::bad_request("the owner cannot leave their team: delete it instead"));
            }
            conn.execute("DELETE FROM team_members WHERE team_id = ?1 AND account_id = ?2", params![team_id, account_id])?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

fn invite_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Invite> {
    let role: String = r.get(2)?;
    Ok(Invite {
        token: r.get(0)?,
        team_id: r.get(1)?,
        role: TeamRole::parse(&role).unwrap_or(TeamRole::Guest),
        created_at: r.get(3)?,
    })
}

pub async fn list_invites(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Json<Vec<Invite>>> {
    auth.user.check_manage(id)?;
    state
        .db(move |_, conn| {
            let mut stmt = conn.prepare(
                "SELECT token, team_id, role, created_at FROM team_invites WHERE team_id = ?1 ORDER BY created_at, token",
            )?;
            Ok(Json(stmt.query_map([id], invite_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?))
        })
        .await
}

pub async fn create_invite(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<CreateInvite>,
) -> Result<Json<Invite>> {
    auth.user.check_manage(id)?;
    grantable(body.role)?;
    let mut bytes = [0u8; 12];
    rand::rng().fill(&mut bytes);
    let token = hex::encode(bytes);
    state
        .db(move |_, conn| {
            if !db::team_exists(conn, id)? {
                return Err(Error::not_found());
            }
            conn.execute(
                "INSERT INTO team_invites (token, team_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![token, id, body.role.as_str(), db::now()],
            )?;
            Ok(Json(conn.query_row(
                "SELECT token, team_id, role, created_at FROM team_invites WHERE token = ?1",
                [&token],
                invite_from_row,
            )?))
        })
        .await
}

pub async fn delete_invite(State(state): State<AppState>, auth: Auth, Path(token): Path<String>) -> Result<StatusCode> {
    state
        .db(move |_, conn| {
            let team: Option<i64> =
                conn.query_row("SELECT team_id FROM team_invites WHERE token = ?1", [&token], |r| r.get(0)).optional()?;
            let team = team.ok_or_else(Error::not_found)?;
            auth.user.check_manage(team)?;
            conn.execute("DELETE FROM team_invites WHERE token = ?1", [&token])?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

/// Join a team through a share link; returns the team. An existing member
/// keeps the better of their role and the link's.
pub fn join(conn: &Connection, account_id: i64, token: &str) -> Result<Option<i64>> {
    let invite: Option<(i64, String)> = conn
        .query_row("SELECT team_id, role FROM team_invites WHERE token = ?1", [token], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((team_id, role)) = invite else { return Ok(None) };
    let role = TeamRole::parse(&role).unwrap_or(TeamRole::Guest);
    let existing: Option<String> = conn
        .query_row(
            "SELECT role FROM team_members WHERE team_id = ?1 AND account_id = ?2",
            params![team_id, account_id],
            |r| r.get(0),
        )
        .optional()?;
    match existing.as_deref().and_then(TeamRole::parse) {
        Some(current) if current >= role => {}
        Some(_) => {
            conn.execute(
                "UPDATE team_members SET role = ?1 WHERE team_id = ?2 AND account_id = ?3",
                params![role.as_str(), team_id, account_id],
            )?;
        }
        None => {
            conn.execute(
                "INSERT INTO team_members (team_id, account_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![team_id, account_id, role.as_str(), db::now()],
            )?;
        }
    }
    Ok(Some(team_id))
}
