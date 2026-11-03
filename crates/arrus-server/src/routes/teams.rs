//! Admin: teams and their members.

use arrus_api::{ADMIN_USERNAME, CreateMember, CreateTeam, Member, Team, UpdateMember, UpdateTeam};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};

use super::ApiJson;
use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::{AppState, db, storage, words};

const TEAM_SELECT: &str = "SELECT t.id, t.name, \
     (SELECT COUNT(*) FROM members m WHERE m.team_id = t.id), \
     (SELECT COUNT(*) FROM choreographies c WHERE c.team_id = t.id), \
     (SELECT COALESCE(SUM(c.size_bytes + COALESCE(c.music_bytes, 0)), 0) FROM choreographies c WHERE c.team_id = t.id) \
     FROM teams t";

fn team_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Team> {
    Ok(Team {
        id: r.get(0)?,
        name: r.get(1)?,
        member_count: r.get::<_, i64>(2)? as u32,
        choreography_count: r.get::<_, i64>(3)? as u32,
        bytes_used: r.get::<_, i64>(4)? as u64,
    })
}

fn load_team(conn: &Connection, id: i64) -> Result<Team> {
    conn.query_row(&format!("{TEAM_SELECT} WHERE t.id = ?1"), [id], team_from_row)
        .optional()?
        .ok_or_else(Error::not_found)
}

fn clean_team_name(conn: &Connection, name: &str, except: Option<i64>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(Error::bad_request("team name must be 1 to 80 characters"));
    }
    let taken: Option<i64> = conn
        .query_row(
            "SELECT id FROM teams WHERE name = ?1 COLLATE NOCASE AND id IS NOT ?2",
            params![name, except],
            |r| r.get(0),
        )
        .optional()?;
    if taken.is_some() {
        return Err(Error::with_code(StatusCode::CONFLICT, "name_taken", "a team with that name already exists"));
    }
    Ok(name.to_string())
}

/// Trimmed, 1–40 chars of letters/digits/`._-`, not `admin`.
pub fn clean_username(username: &str) -> Result<String> {
    let u = username.trim();
    if u.is_empty() || u.chars().count() > 40 {
        return Err(Error::bad_request("username must be 1 to 40 characters"));
    }
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(Error::bad_request("username may only contain letters, digits, '.', '_' and '-'"));
    }
    if u.eq_ignore_ascii_case(ADMIN_USERNAME) {
        return Err(Error::with_code(StatusCode::CONFLICT, "username_taken", "that username is reserved"));
    }
    Ok(u.to_string())
}

fn username_taken(conn: &Connection, username: &str, except: Option<i64>) -> Result<()> {
    let taken: Option<i64> = conn
        .query_row(
            "SELECT id FROM members WHERE username_lower = ?1 AND id IS NOT ?2",
            params![username.to_lowercase(), except],
            |r| r.get(0),
        )
        .optional()?;
    if taken.is_some() {
        return Err(Error::with_code(StatusCode::CONFLICT, "username_taken", "that username is already taken"));
    }
    Ok(())
}

fn member_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Member> {
    Ok(Member { id: r.get(0)?, team_id: r.get(1)?, username: r.get(2)?, password: r.get(3)?, can_write: r.get(4)? })
}

const MEMBER_SELECT: &str = "SELECT id, team_id, username, password, can_write FROM members";

pub async fn list(State(state): State<AppState>, auth: Auth) -> Result<Json<Vec<Team>>> {
    auth.user.require_admin()?;
    state
        .db(|_, conn| {
            let mut stmt = conn.prepare(&format!("{TEAM_SELECT} ORDER BY t.name COLLATE NOCASE"))?;
            let teams = stmt.query_map([], team_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Json(teams))
        })
        .await
}

pub async fn create(State(state): State<AppState>, auth: Auth, ApiJson(body): ApiJson<CreateTeam>) -> Result<Json<Team>> {
    auth.user.require_admin()?;
    state
        .db(move |_, conn| {
            let name = clean_team_name(conn, &body.name, None)?;
            conn.execute("INSERT INTO teams (name, created_at) VALUES (?1, ?2)", params![name, db::now()])?;
            Ok(Json(load_team(conn, conn.last_insert_rowid())?))
        })
        .await
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<UpdateTeam>,
) -> Result<Json<Team>> {
    auth.user.require_admin()?;
    state
        .db(move |_, conn| {
            load_team(conn, id)?;
            let name = clean_team_name(conn, &body.name, Some(id))?;
            conn.execute("UPDATE teams SET name = ?1 WHERE id = ?2", params![name, id])?;
            Ok(Json(load_team(conn, id)?))
        })
        .await
}

pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<StatusCode> {
    auth.user.require_admin()?;
    state
        .db(move |s, conn| {
            let tx = conn.transaction()?;
            if !db::team_exists(&tx, id)? {
                return Err(Error::not_found());
            }
            let files: Vec<String> = {
                let mut stmt =
                    tx.prepare("SELECT music_file FROM choreographies WHERE team_id = ?1 AND music_file IS NOT NULL")?;
                stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            // Members (and so their sessions) and choreographies cascade.
            tx.execute("DELETE FROM teams WHERE id = ?1", [id])?;
            tx.commit()?;
            storage::remove_music_files(s, files);
            storage::after_change(s, conn)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

pub async fn list_members(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Json<Vec<Member>>> {
    auth.user.require_admin()?;
    state
        .db(move |_, conn| {
            if !db::team_exists(conn, id)? {
                return Err(Error::not_found());
            }
            let mut stmt = conn.prepare(&format!("{MEMBER_SELECT} WHERE team_id = ?1 ORDER BY username_lower"))?;
            let members = stmt.query_map([id], member_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Json(members))
        })
        .await
}

pub async fn create_member(
    State(state): State<AppState>,
    auth: Auth,
    Path(team_id): Path<i64>,
    ApiJson(body): ApiJson<CreateMember>,
) -> Result<Json<Member>> {
    auth.user.require_admin()?;
    let username = clean_username(&body.username)?;
    state
        .db(move |_, conn| {
            if !db::team_exists(conn, team_id)? {
                return Err(Error::not_found());
            }
            username_taken(conn, &username, None)?;
            conn.execute(
                "INSERT INTO members (team_id, username, username_lower, password, can_write, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![team_id, username, username.to_lowercase(), words::generate_password(), body.can_write, db::now()],
            )?;
            let id = conn.last_insert_rowid();
            Ok(Json(conn.query_row(&format!("{MEMBER_SELECT} WHERE id = ?1"), [id], member_from_row)?))
        })
        .await
}

pub async fn update_member(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<UpdateMember>,
) -> Result<Json<Member>> {
    auth.user.require_admin()?;
    let username = body.username.as_deref().map(clean_username).transpose()?;
    state
        .db(move |_, conn| {
            let tx = conn.transaction()?;
            let exists: Option<i64> = tx.query_row("SELECT id FROM members WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            if exists.is_none() {
                return Err(Error::not_found());
            }
            if let Some(u) = &username {
                username_taken(&tx, u, Some(id))?;
                tx.execute(
                    "UPDATE members SET username = ?1, username_lower = ?2 WHERE id = ?3",
                    params![u, u.to_lowercase(), id],
                )?;
            }
            if let Some(w) = body.can_write {
                tx.execute("UPDATE members SET can_write = ?1 WHERE id = ?2", params![w, id])?;
            }
            if body.regenerate_password {
                tx.execute("UPDATE members SET password = ?1 WHERE id = ?2", params![words::generate_password(), id])?;
                tx.execute("DELETE FROM sessions WHERE member_id = ?1", [id])?;
            }
            let m = tx.query_row(&format!("{MEMBER_SELECT} WHERE id = ?1"), [id], member_from_row)?;
            tx.commit()?;
            Ok(Json(m))
        })
        .await
}

pub async fn delete_member(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<StatusCode> {
    auth.user.require_admin()?;
    state
        .db(move |_, conn| {
            // Sessions cascade.
            let n = conn.execute("DELETE FROM members WHERE id = ?1", [id])?;
            if n == 0 {
                return Err(Error::not_found());
            }
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}
