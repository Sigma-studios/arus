//! Choreography CRUD.

use arrus_api::{
    ChoreographyDoc, ChoreographySummary, Conflict, CreateChoreography, SaveChoreography, Saved,
};
use arrus_core::Choreography;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use super::ApiJson;
use crate::auth::{Auth, User};
use crate::error::{Error, Result};
use crate::{AppState, db, storage};

/// A choreography serialized for storage.
pub struct Stored {
    pub json: String,
    pub name: String,
    pub formation_count: i64,
    pub dancer_count: i64,
    pub uuid: String,
    pub creation_timestamp: i64,
}

impl Stored {
    pub fn new(c: &Choreography) -> Result<Self> {
        Ok(Self {
            json: serde_json::to_string(c).map_err(Error::internal)?,
            name: c.name.clone(),
            formation_count: c.formations.len() as i64,
            dancer_count: c.dancer_count() as i64,
            uuid: c.id.clone(),
            creation_timestamp: c.creation_timestamp,
        })
    }
    pub fn size(&self) -> u64 {
        self.json.len() as u64
    }
}

/// Insert a new row (quota already checked); returns its id.
pub fn insert(conn: &Connection, team_id: i64, s: &Stored, by: &str, now: i64) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO choreographies (team_id, name, data, size_bytes, formation_count, dancer_count, uuid, \
         creation_timestamp, revision, created_at, updated_at, updated_by) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9, ?10)",
        params![
            team_id,
            s.name,
            s.json,
            s.size() as i64,
            s.formation_count,
            s.dancer_count,
            s.uuid,
            s.creation_timestamp,
            now,
            by
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The choreography's metadata if `user` may read it (404 otherwise).
pub fn readable(conn: &Connection, user: &User, id: i64) -> Result<db::ChoreoMeta> {
    let meta = db::choreo_meta(conn, id)?.ok_or_else(Error::not_found)?;
    user.check_read(meta.team_id)?;
    Ok(meta)
}

/// The choreography's metadata if `user` may change it (404 / 403 otherwise).
pub fn writable(conn: &Connection, user: &User, id: i64) -> Result<db::ChoreoMeta> {
    let meta = db::choreo_meta(conn, id)?.ok_or_else(Error::not_found)?;
    user.check_write(meta.team_id)?;
    Ok(meta)
}

pub fn summary(conn: &Connection, id: i64) -> Result<ChoreographySummary> {
    db::summary(conn, id)?.ok_or_else(Error::not_found)
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    team: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ChoreographySummary>>> {
    let team_id = match (&auth.user, q.team) {
        (User::Admin, Some(t)) => t,
        (User::Admin, None) => return Err(Error::bad_request("the admin must pass ?team=<id>")),
        (User::Member { team_id, .. }, Some(t)) if t != *team_id => return Err(Error::not_found()),
        (User::Member { team_id, .. }, _) => *team_id,
    };
    state
        .db(move |_, conn| {
            if !db::team_exists(conn, team_id)? {
                return Err(Error::not_found());
            }
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM choreographies WHERE team_id = ?1 ORDER BY updated_at DESC, id DESC",
                db::SUMMARY_COLS
            ))?;
            let list = stmt.query_map([team_id], db::summary_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Json(list))
        })
        .await
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(body): ApiJson<CreateChoreography>,
) -> Result<Json<ChoreographySummary>> {
    let team_id = match (&auth.user, body.team_id) {
        (User::Admin, Some(t)) => t,
        (User::Admin, None) => return Err(Error::bad_request("the admin must pass team_id")),
        (User::Member { team_id, .. }, _) => *team_id,
    };
    auth.user.check_write(team_id)?;
    let now = db::now();
    let mut data = body.data;
    data.id = uuid::Uuid::new_v4().to_string();
    data.creation_timestamp = now;
    data.last_edit_timestamp = now;
    let stored = Stored::new(&data)?;
    let by = auth.user.username();
    state
        .db(move |s, conn| {
            if !db::team_exists(conn, team_id)? {
                return Err(Error::not_found());
            }
            storage::ensure_fits(s, conn, 0, stored.size())?;
            let id = insert(conn, team_id, &stored, &by, now)?;
            storage::after_change(s, conn)?;
            Ok(Json(summary(conn, id)?))
        })
        .await
}

pub async fn get(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Json<ChoreographyDoc>> {
    let (summary, json) = state
        .db(move |_, conn| {
            readable(conn, &auth.user, id)?;
            let json: Option<String> =
                conn.query_row("SELECT data FROM choreographies WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            Ok((summary(conn, id)?, json.ok_or_else(Error::not_found)?))
        })
        .await?;
    let data: Choreography = serde_json::from_str(&json).map_err(Error::internal)?;
    Ok(Json(ChoreographyDoc { summary, data }))
}

pub async fn save(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<SaveChoreography>,
) -> Result<Json<Saved>> {
    let by = auth.user.username();
    let mut data = body.data;
    state
        .db(move |s, conn| {
            let meta = writable(conn, &auth.user, id)?;
            if meta.revision != body.base_revision {
                return Err(Error::Conflict(Conflict {
                    current_revision: meta.revision,
                    updated_by: meta.updated_by,
                    updated_at: meta.updated_at,
                }));
            }
            let now = db::now();
            // The app's identity of the choreography never changes on save.
            data.id = meta.uuid.clone();
            data.creation_timestamp = meta.creation_timestamp;
            data.last_edit_timestamp = now;
            let stored = Stored::new(&data)?;
            storage::ensure_fits(s, conn, meta.size_bytes, stored.size())?;
            conn.execute(
                "UPDATE choreographies SET name = ?1, data = ?2, size_bytes = ?3, formation_count = ?4, \
                 dancer_count = ?5, revision = revision + 1, updated_at = ?6, updated_by = ?7 WHERE id = ?8",
                params![
                    stored.name,
                    stored.json,
                    stored.size() as i64,
                    stored.formation_count,
                    stored.dancer_count,
                    now,
                    by,
                    id
                ],
            )?;
            storage::after_change(s, conn)?;
            Ok(Json(Saved { summary: summary(conn, id)? }))
        })
        .await
}

pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<StatusCode> {
    state
        .db(move |s, conn| {
            let meta = writable(conn, &auth.user, id)?;
            conn.execute("DELETE FROM choreographies WHERE id = ?1", [id])?;
            storage::remove_music_files(s, meta.music_file);
            storage::after_change(s, conn)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}
