//! Choreography CRUD, and live co-editing (patches and the event stream).

use std::convert::Infallible;
use std::time::Duration;

use arrus_api::{
    ChoreographyDoc, ChoreographySummary, Conflict, CreateChoreography, LiveEvent, MAX_PRESENCE_SELECTED, MAX_SESSION_LEN,
    PatchChoreography, PresenceUpdate, SaveChoreography, Saved, Viewer, patch as jpatch,
};
use arrus_core::Choreography;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
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

/// Store `data` as the next revision of choreography `id` (write access and
/// quota checked by the caller / here).
fn store(
    s: &AppState,
    conn: &Connection,
    id: i64,
    meta: &db::ChoreoMeta,
    mut data: Choreography,
    by: &str,
) -> Result<ChoreographySummary> {
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
        params![stored.name, stored.json, stored.size() as i64, stored.formation_count, stored.dancer_count, now, by, id],
    )?;
    storage::after_change(s, conn)?;
    summary(conn, id)
}

pub async fn save(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<SaveChoreography>,
) -> Result<Json<Saved>> {
    let by = auth.user.username();
    let data = body.data;
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
            let summary = store(s, conn, id, &meta, data, &by)?;
            // Live viewers see a new revision they cannot patch their way to: they reload.
            s.hub.send(id, &LiveEvent::Hello { revision: summary.revision });
            Ok(Json(Saved { summary }))
        })
        .await
}

/// Apply a batch of changes to the latest version and pass it on to everyone
/// viewing the choreography.
pub async fn patch(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<PatchChoreography>,
) -> Result<Json<Saved>> {
    if body.session.is_empty() || body.session.len() > MAX_SESSION_LEN {
        return Err(Error::bad_request("bad session id"));
    }
    let by = auth.user.username();
    state
        .db(move |s, conn| {
            let meta = writable(conn, &auth.user, id)?;
            if body.ops.is_empty() {
                return Ok(Json(Saved { summary: summary(conn, id)? }));
            }
            let json: String = conn.query_row("SELECT data FROM choreographies WHERE id = ?1", [id], |r| r.get(0))?;
            let mut value: serde_json::Value = serde_json::from_str(&json).map_err(Error::internal)?;
            jpatch::apply(&mut value, &body.ops);
            let data: Choreography = serde_json::from_value(value)
                .map_err(|e| Error::bad_request(format!("this change would break the choreography: {e}")))?;
            let summary = store(s, conn, id, &meta, data, &by)?;
            // Sent under the database lock: viewers get revisions in order.
            s.hub.send(
                id,
                &LiveEvent::Patch { revision: summary.revision, session: body.session, batch: body.batch, by, ops: body.ops },
            );
            Ok(Json(Saved { summary }))
        })
        .await
}

/// Where an editor session is (formation, selected dancers): passed on to
/// everyone viewing. 404 if that session has no live stream open.
pub async fn presence(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<PresenceUpdate>,
) -> Result<StatusCode> {
    if body.session.is_empty() || body.session.len() > MAX_SESSION_LEN || body.selected.len() > MAX_PRESENCE_SELECTED {
        return Err(Error::bad_request("bad presence update"));
    }
    let user = auth.user.clone();
    state.db(move |_, conn| readable(conn, &user, id).map(|_| ())).await?;
    let mine = state.hub.viewers(id).iter().any(|v| v.session == body.session && v.username == auth.user.username());
    if !mine || !state.hub.update(id, &body) {
        return Err(Error::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct LiveQuery {
    session: String,
}

/// The live event stream of a choreography (server-sent events): a
/// [`LiveEvent::Hello`] first, then presence and changes as they happen.
pub async fn live(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Query(q): Query<LiveQuery>,
) -> Result<Response> {
    if q.session.is_empty()
        || q.session.len() > MAX_SESSION_LEN
        || !q.session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(Error::bad_request("bad session id"));
    }
    let user = auth.user.clone();
    let meta = state.db(move |_, conn| readable(conn, &user, id)).await?;
    let can_write = auth.user.check_write(meta.team_id).is_ok();
    let viewer = Viewer { session: q.session.clone(), username: auth.user.username(), can_write, ..Default::default() };
    // Subscribe before reading the revision, so no change falls in between.
    let (conn, rx) = state.hub.join(id, viewer);
    let leave = crate::live::Leave { state: state.clone(), id, conn };
    let revision = state.db(move |_, conn| Ok(db::choreo_meta(conn, id)?.map_or(0, |m| m.revision))).await?;
    let hello: std::sync::Arc<str> =
        serde_json::to_string(&LiveEvent::Hello { revision }).map_err(Error::internal)?.into();

    let stream = futures_util::stream::unfold((rx, leave, Some(hello)), |(mut rx, leave, first)| async move {
        if let Some(h) = first {
            return Some((Ok::<_, Infallible>(Event::default().data(&*h)), (rx, leave, None)));
        }
        let msg = match rx.recv().await {
            Ok(m) => m,
            // Fell behind: make the client reload the document.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => r#"{"type":"hello","revision":-1}"#.into(),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        };
        Some((Ok(Event::default().data(&*msg)), (rx, leave, None)))
    });
    let mut res = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response();
    // Reverse proxies must pass events through as they come.
    res.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    res.headers_mut().insert(axum::http::header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<StatusCode> {
    state
        .db(move |s, conn| {
            let meta = writable(conn, &auth.user, id)?;
            conn.execute("DELETE FROM choreographies WHERE id = ?1", [id])?;
            s.hub.send(id, &LiveEvent::Deleted);
            storage::remove_music_files(s, meta.music_file);
            storage::after_change(s, conn)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}
