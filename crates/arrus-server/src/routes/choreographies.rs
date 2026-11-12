//! Choreography CRUD, and live co-editing (patches and the event stream).

use std::convert::Infallible;
use std::time::Duration;

use arrus_api::{
    ChoreographyDoc, ChoreographySummary, Conflict, CreateChoreography, LiveEvent, MAX_PRESENCE_SELECTED, MAX_SESSION_LEN,
    LiveRequest, MAX_MOVING, PatchChoreography, PatchOp, PresenceUpdate, SaveChoreography, Saved, Viewer,
};
use arrus_core::Choreography;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use super::ApiJson;
use crate::activity::Kind;
use crate::auth::{Auth, User};
use crate::error::{Error, Result};
use crate::{AppState, db, docs, storage};

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

/// The team a request means: the one it names, or an account's only team.
pub fn team_of(user: &User, named: Option<i64>) -> Result<i64> {
    named.or_else(|| user.default_team()).ok_or_else(|| Error::bad_request("name the team (team_id / ?team=)"))
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
    let team_id = team_of(&auth.user, q.team)?;
    auth.user.check_read(team_id)?;
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
    let team_id = team_of(&auth.user, body.team_id)?;
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
            db::ensure_choreography_room(conn, team_id, 1)?;
            storage::ensure_fits(s, conn, 0, stored.size())?;
            let id = insert(conn, team_id, &stored, &by, now)?;
            storage::after_change(s, conn)?;
            if let Some(a) = auth.user.account_id() {
                s.activity.add(a, Kind::Created, 1);
            }
            Ok(Json(summary(conn, id)?))
        })
        .await
}

pub async fn get(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Json<ChoreographyDoc>> {
    let user = auth.user.clone();
    let (summary, json) = state
        .db(move |_, conn| {
            readable(conn, &user, id)?;
            let json: Option<String> =
                conn.query_row("SELECT data FROM choreographies WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            Ok((summary(conn, id)?, json.ok_or_else(Error::not_found)?))
        })
        .await?;
    let data: Choreography = serde_json::from_str(&json).map_err(Error::internal)?;
    let can_write = auth.user.check_write(summary.team_id).is_ok();
    Ok(Json(ChoreographyDoc { summary, data, can_write }))
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
            if let Some(a) = auth.user.account_id() {
                s.activity.add(a, Kind::Edits, 1);
            }
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
    live_patch(&state, auth.user, id, body.session, body.batch, body.ops).await.map(Json)
}

/// A batch of live changes (from `POST …/patch` or the WebSocket): applied
/// in memory (see [`crate::docs`]) and sent to everyone viewing.
async fn live_patch(state: &AppState, user: User, id: i64, session: String, batch: u64, ops: Vec<PatchOp>) -> Result<Saved> {
    let by = user.username();
    state
        .db_live(move |s, conn| {
            let meta = writable(conn, &user, id)?;
            if ops.is_empty() {
                return Ok(Saved { summary: docs::summary(s, conn, id)? });
            }
            let summary = docs::apply_patch(s, conn, &meta, &ops, &by)?;
            if let Some(a) = user.account_id() {
                s.activity.add(a, Kind::Edits, 1);
            }
            // Sent under the database lock: viewers get revisions in order.
            s.hub.send(id, &LiveEvent::Patch { revision: summary.revision, session, batch, by, ops });
            Ok(Saved { summary })
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
    state.db_live(move |_, conn| readable(conn, &user, id).map(|_| ())).await?;
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

/// Live co-editing over one WebSocket: the [`LiveEvent`]s of
/// [`live`], plus answers to the [`LiveRequest`]s the editor sends (patches,
/// presence, dancers being dragged). Login and access are checked when it
/// opens; each patch checks the session and write access again.
pub async fn ws(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Query(q): Query<LiveQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response> {
    if !valid_session(&q.session) {
        return Err(Error::bad_request("bad session id"));
    }
    // Browsers send cookies with WebSocket requests from other sites: only
    // accept our own pages.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && !same_site(&state, &headers, origin)
    {
        return Err(Error::forbidden("cross-site WebSocket"));
    }
    let user = auth.user.clone();
    let meta = state.db_live(move |_, conn| readable(conn, &user, id)).await?;
    let can_write = auth.user.check_write(meta.team_id).is_ok();
    Ok(upgrade
        .max_message_size(super::JSON_LIMIT)
        .on_upgrade(move |socket| live_socket(state, auth, id, q.session, can_write, socket)))
}

/// `origin` is this site: the host asked for (directly or through a
/// reverse proxy), or the configured public URL.
fn same_site(state: &AppState, headers: &HeaderMap, origin: &str) -> bool {
    let origin_host = origin.split_once("://").map_or(origin, |(_, h)| h);
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let hosts = [header("host"), header("x-forwarded-host").and_then(|h| h.split(',').next()).map(str::trim)];
    hosts.into_iter().flatten().any(|h| h.eq_ignore_ascii_case(origin_host))
        || state.config.public_url.as_deref().is_some_and(|u| u.trim_end_matches('/').eq_ignore_ascii_case(origin))
}

fn valid_session(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_SESSION_LEN && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Shortest time between two [`LiveRequest::Moving`] passed on from one socket.
const MOVING_EVERY: Duration = Duration::from_millis(40);
/// Pings keep proxies from closing a quiet socket.
const PING_EVERY: Duration = Duration::from_secs(25);

async fn live_socket(state: AppState, auth: Auth, id: i64, session: String, can_write: bool, mut socket: WebSocket) {
    let username = auth.user.username();
    let viewer = Viewer { session: session.clone(), username: username.clone(), can_write, ..Default::default() };
    // Subscribe before reading the revision, so no change falls in between.
    let (conn, mut rx) = state.hub.join(id, viewer);
    let _leave = crate::live::Leave { state: state.clone(), id, conn };
    let Ok(revision) = state.db(move |s, conn| Ok(docs::summary(s, conn, id)?.revision)).await else { return };
    if send_event(&mut socket, &LiveEvent::Hello { revision }).await.is_err() {
        return;
    }
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_moving: Option<tokio::time::Instant> = None;
    // Our own dragged dancers are not sent back (the tag and session come first).
    let own_moving = format!(r#"{{"type":"moving","session":{}"#, serde_json::Value::from(session.clone()));
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let text: std::sync::Arc<str> = match msg {
                    Ok(m) => m,
                    // Fell behind: make the client reload the document.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => r#"{"type":"hello","revision":-1}"#.into(),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if text.starts_with(&own_moving) {
                    continue;
                }
                if socket.send(Message::Text((&*text).into())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                let Ok(req) = serde_json::from_str::<LiveRequest>(&text) else { continue };
                if let Some(a) = auth.user.account_id() {
                    state.activity.seen(a);
                }
                let reply = match req {
                    LiveRequest::Patch { batch, ops } => {
                        // Logged out or removed from the team since the socket opened?
                        let token = auth.token.clone();
                        let user = state
                            .db_live(move |_, conn| Ok(crate::auth::lookup(conn, &token)?))
                            .await
                            .ok()
                            .flatten()
                            .map(|a| a.user);
                        let r = match user {
                            Some(user) => live_patch(&state, user, id, session.clone(), batch, ops).await,
                            None => Err(Error::unauthorized()),
                        };
                        Some(match r {
                            Ok(saved) => LiveEvent::Saved { batch, summary: saved.summary },
                            Err(e) => LiveEvent::Refused { batch, status: e.status().as_u16(), error: e.api_error() },
                        })
                    }
                    LiveRequest::Presence { formation, selected, dragging } => {
                        if selected.len() <= MAX_PRESENCE_SELECTED {
                            state.hub.update(id, &PresenceUpdate { session: session.clone(), formation, selected, dragging });
                        }
                        None
                    }
                    LiveRequest::Moving { formation, dancers } => {
                        let now = tokio::time::Instant::now();
                        if can_write && dancers.len() <= MAX_MOVING && last_moving.is_none_or(|t| now - t >= MOVING_EVERY) {
                            last_moving = Some(now);
                            state.hub.send(id, &LiveEvent::Moving { session: session.clone(), formation, dancers });
                        }
                        None
                    }
                };
                if let Some(ev) = reply && send_event(&mut socket, &ev).await.is_err() {
                    break;
                }
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, ev: &LiveEvent) -> Result<(), axum::Error> {
    socket.send(Message::Text(serde_json::to_string(ev).unwrap_or_default().into())).await
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
