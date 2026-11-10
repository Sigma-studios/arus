//! Live co-editing in the editor, like a shared spreadsheet: every change is
//! saved automatically as a small patch, other people's changes appear as
//! they make them, and everyone with the choreography open is listed.
//!
//! The client keeps the server's document as it last heard of it (`server`)
//! and that plus its own unconfirmed batches (`shadow`). New local changes
//! are `diff(shadow, doc)`. The server applies batches in arrival order and
//! streams them back, numbered by revision; a gap in the numbers (missed
//! messages, a reconnect) makes the client reload the document and replay
//! its unconfirmed batches on top. Batches are safe to resend.
//!
//! Everything goes over one WebSocket: changes, where we are, and the dancers
//! being dragged (streamed so others see them move, stored only on drop).
//! Where the WebSocket cannot get through (it never opens), the editor falls
//! back to the server-sent event stream plus one POST per batch / presence.

use std::collections::VecDeque;
use std::rc::Rc;

use arrus_api::patch::{self, IMAGE_PATH, PatchOp};
use arrus_api::{
    ChoreographyDoc, ChoreographyId, LiveEvent, LiveRequest, MovingDancer, PatchChoreography, PresenceUpdate, Saved, Viewer,
};
use arrus_core::Choreography;
use serde_json::Value;

use super::editor::Editor;
use super::stage::Drag;
use super::{ApiError, App, net};
use crate::web;

/// Quiet time after the last change before it is sent…
const DEBOUNCE: f64 = 0.25;
/// …but during a continuous change (a drag) send at least this often.
const MAX_DELAY: f64 = 0.5;
/// While dragging dancers over the WebSocket the drag is streamed instead
/// (see [`MOVING_EVERY`]), and stored on drop or at least this often.
const DRAG_MAX_DELAY: f64 = 5.0;
/// Where we are (formation, selection) is reported at most this often.
const PRESENCE_EVERY: f64 = 0.15;
/// Dragged dancers are streamed this often.
const MOVING_EVERY: f64 = 0.1;
/// Wait before opening a new WebSocket after one closed.
const RECONNECT_SECS: f64 = 3.0;
/// WebSockets that never open, in a row, before using the fallback.
const SOCKET_TRIES: u32 = 2;

struct Batch {
    id: u64,
    ops: Vec<PatchOp>,
    /// Revision the server stored it as, once it said so.
    acked_rev: Option<i64>,
}

pub struct Live {
    pub session: String,
    /// The server's document as last heard of (without the background image).
    server: Value,
    pub server_rev: i64,
    /// `server` with our unconfirmed batches applied.
    shadow: Value,
    unacked: VecDeque<Batch>,
    next_batch: u64,
    inflight: Option<u64>,
    /// The background image as last sent or received.
    bg_sent: Option<Rc<String>>,
    /// Local changes not yet turned into a batch.
    changed: bool,
    last_change: f64,
    last_collect: f64,
    retry_at: f64,
    failures: u32,
    /// Saving failed for lack of a connection; retrying.
    pub offline: bool,
    pub viewers: Vec<Viewer>,
    /// The live WebSocket, while connecting or open.
    socket: Option<web::Socket>,
    socket_open: bool,
    socket_closed_at: Option<f64>,
    socket_ever_opened: bool,
    /// Sockets that closed without opening, in a row.
    socket_failures: u32,
    /// The WebSocket does not get through: event stream + POSTs instead.
    pub fallback: bool,
    /// Dragged dancers last streamed (formation, dancers) and when.
    moving_sent: Option<(usize, Vec<MovingDancer>)>,
    moving_at: f64,
    stream: Option<web::EventStream>,
    stream_dead_since: Option<f64>,
    resyncing: bool,
    /// Other people's changes, in order. Undo snapshots replay the ones made
    /// after them, so undo only takes back your own changes.
    pub remote_log: Vec<Rc<Vec<PatchOp>>>,
    /// The live stream said hello (so the server knows this session).
    connected: bool,
    /// Where we last told the others we are.
    presence_sent: Option<PresenceUpdate>,
    presence_at: f64,
    presence_busy: bool,
}

fn value_of(doc: &Choreography) -> Value {
    serde_json::to_value(doc).unwrap_or(Value::Null)
}

fn same_image(a: &Option<Rc<String>>, b: &Option<Rc<String>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b) || a == b,
        (None, None) => true,
        _ => false,
    }
}

impl Live {
    pub fn new(doc: &Choreography, revision: i64, bg: Option<Rc<String>>) -> Self {
        let server = value_of(doc);
        Self {
            session: web::random_uuid(),
            shadow: server.clone(),
            server,
            server_rev: revision,
            unacked: VecDeque::new(),
            next_batch: 1,
            inflight: None,
            bg_sent: bg,
            changed: false,
            last_change: 0.0,
            last_collect: 0.0,
            retry_at: 0.0,
            failures: 0,
            offline: false,
            viewers: Vec::new(),
            socket: None,
            socket_open: false,
            socket_closed_at: None,
            socket_ever_opened: false,
            socket_failures: 0,
            fallback: false,
            moving_sent: None,
            moving_at: 0.0,
            stream: None,
            stream_dead_since: None,
            resyncing: false,
            remote_log: Vec::new(),
            connected: false,
            presence_sent: None,
            presence_at: 0.0,
            presence_busy: false,
        }
    }

    pub fn note_change(&mut self, now: f64) {
        if !self.changed {
            self.last_collect = now;
        }
        self.changed = true;
        self.last_change = now;
    }

    /// Changes not yet stored by the server.
    pub fn unsaved(&self) -> bool {
        self.changed || self.unacked.iter().any(|b| b.acked_rev.is_none())
    }

    pub fn saving(&self) -> bool {
        self.inflight.is_some() || self.unsaved()
    }

    /// Everyone else with the choreography open, one entry per person.
    pub fn others(&self) -> Vec<&Viewer> {
        let mut out: Vec<&Viewer> = Vec::new();
        for v in self.viewers.iter().filter(|v| v.session != self.session) {
            match out.iter_mut().find(|o| o.username == v.username) {
                Some(o) if v.can_write && !o.can_write => *o = v,
                Some(_) => {}
                None => out.push(v),
            }
        }
        out
    }

    /// The WebSocket ended (`opened`: after it had opened).
    fn socket_closed(&mut self, now: f64, opened: bool) {
        self.socket = None;
        self.socket_open = false;
        self.socket_closed_at = Some(now);
        self.connected = false;
        self.viewers.clear();
        self.presence_sent = None;
        self.moving_sent = None;
        // An answer that will never come: send the batch again once back.
        self.inflight = None;
        if !opened {
            self.socket_failures += 1;
            if !self.socket_ever_opened && self.socket_failures >= SOCKET_TRIES {
                self.fallback = true;
            }
        }
        self.offline = self.unsaved();
    }

    /// Connected: changes we send will come back confirmed.
    fn link_up(&self) -> bool {
        if self.fallback { self.stream.is_some() && self.stream_dead_since.is_none() } else { self.socket_open }
    }

    fn send(&self, req: &LiveRequest) -> bool {
        match (&self.socket, self.socket_open) {
            (Some(s), true) => serde_json::to_string(req).is_ok_and(|t| s.send(&t)),
            _ => false,
        }
    }

    fn has_image_op(&self) -> bool {
        self.unacked.iter().any(|b| b.ops.iter().any(patch::touches_image))
    }
}

impl Editor {
    /// Turn local changes into a batch (or add them to the one waiting).
    pub fn collect(&mut self) {
        let now = self.now;
        let live = &mut self.live;
        live.changed = false;
        live.last_collect = now;
        if !self.can_edit {
            return;
        }
        let v = value_of(&self.doc);
        let mut ops = patch::diff(&live.shadow, &v);
        if !same_image(&self.bg_data, &live.bg_sent) {
            let path = IMAGE_PATH.iter().map(|k| (*k).into()).collect();
            let value = self.bg_data.as_ref().map_or(Value::Null, |s| Value::String((**s).clone()));
            ops.push(PatchOp::Set { path, value });
            live.bg_sent = self.bg_data.clone();
        }
        if ops.is_empty() {
            return;
        }
        live.shadow = v;
        match live.unacked.back_mut() {
            Some(b) if live.inflight != Some(b.id) && b.acked_rev.is_none() => b.ops.extend(ops),
            _ => {
                live.unacked.push_back(Batch { id: live.next_batch, ops, acked_rev: None });
                live.next_batch += 1;
            }
        }
    }

    /// Rebuild the document from the server's version plus our unconfirmed
    /// batches (after someone else's change or a reload).
    fn rebuild(&mut self) -> bool {
        let live = &mut self.live;
        let mut shadow = live.server.clone();
        for b in &live.unacked {
            patch::apply(&mut shadow, &b.ops);
        }
        patch::take_image(&mut shadow);
        match serde_json::from_value::<Choreography>(shadow.clone()) {
            Ok(mut d) => {
                if d.formations.is_empty() {
                    d.formations.push(arrus_core::Formation::new());
                }
                live.shadow = shadow;
                self.doc = d;
                self.after_outside_change();
                true
            }
            Err(_) => false,
        }
    }

    /// An undo snapshot brought up to date with other people's later changes.
    pub fn replay_remote(&self, doc: Choreography, seq: usize) -> Choreography {
        let log = &self.live.remote_log;
        if seq >= log.len() {
            return doc;
        }
        let mut v = value_of(&doc);
        for ops in &log[seq..] {
            patch::apply(&mut v, ops);
        }
        patch::take_image(&mut v);
        serde_json::from_value(v).unwrap_or(doc)
    }
}

/// Every frame: keep the live stream open and send pending changes.
pub fn sync(ed: &mut Editor, app: &mut App) {
    let id = ed.summary.id;
    let now = ed.now;
    if ed.live.fallback {
        open_stream(ed, app, id, now);
    } else {
        open_socket(ed, app, id, now);
    }
    send_presence(ed, app, id, now);
    send_moving(ed, app, now);
    // A drag changes the document every frame without going through
    // `Editor::edit`: keep counting it as changed (sent at least every
    // MAX_DELAY), and once more when it ends, however it ends.
    if ed.gesture {
        if ed.drag.is_none() {
            ed.end_gesture();
        } else {
            ed.live.note_change(now);
        }
    }
    let streaming = !ed.live.fallback && matches!(ed.drag, Some(Drag::Dancers { .. }));
    let max_delay = if streaming { DRAG_MAX_DELAY } else { MAX_DELAY };
    if ed.live.changed && (now - ed.live.last_change >= DEBOUNCE || now - ed.live.last_collect >= max_delay) {
        ed.collect();
    }
    ed.dirty = ed.live.unsaved();
    if ed.live.changed || ed.live.offline {
        app.net.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    }
    if !ed.can_edit || ed.live.inflight.is_some() || now < ed.live.retry_at {
        return;
    }
    let Some(b) = ed.live.unacked.iter().find(|b| b.acked_rev.is_none()) else { return };
    let batch = b.id;
    if !ed.live.fallback {
        // Answered with `Saved` / `Refused` on the socket.
        if ed.live.send(&LiveRequest::Patch { batch, ops: b.ops.clone() }) {
            ed.live.inflight = Some(batch);
        }
        return;
    }
    let body = PatchChoreography { session: ed.live.session.clone(), batch, ops: b.ops.clone() };
    ed.live.inflight = Some(batch);
    app.net.spawn(async move {
        let r = net::send::<_, Saved>("POST", &format!("/api/choreographies/{id}/patch"), &body).await;
        move |app: &mut App| on_sent(app, id, batch, r)
    });
}

/// Tell the others which formation we are on and which dancers we hold.
fn send_presence(ed: &mut Editor, app: &mut App, id: ChoreographyId, now: f64) {
    let here = PresenceUpdate {
        session: ed.live.session.clone(),
        formation: Some(ed.cur()),
        selected: ed.sel.dancers().map(|s| s.iter().copied().collect()).unwrap_or_default(),
        dragging: matches!(ed.drag, Some(Drag::Dancers { moved: true, .. })),
    };
    let live = &mut ed.live;
    if !live.connected || live.presence_sent.as_ref() == Some(&here) {
        return;
    }
    if live.presence_busy || now - live.presence_at < PRESENCE_EVERY {
        app.net.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        return;
    }
    if !live.fallback {
        let req = LiveRequest::Presence { formation: here.formation, selected: here.selected.clone(), dragging: here.dragging };
        if live.send(&req) {
            live.presence_at = now;
            live.presence_sent = Some(here);
        }
        return;
    }
    live.presence_busy = true;
    live.presence_at = now;
    live.presence_sent = Some(here.clone());
    app.net.spawn(async move {
        let r = net::send_empty_json("POST", &format!("/api/choreographies/{id}/presence"), &here).await;
        move |app: &mut App| {
            if let Some(ed) = editor_for(app, id) {
                ed.live.presence_busy = false;
                if r.is_err() {
                    // Not registered yet (reconnecting): try again later.
                    ed.live.presence_sent = None;
                }
            }
        }
    });
}

/// While dragging dancers: stream where they are, so the others see them move.
fn send_moving(ed: &mut Editor, app: &mut App, now: f64) {
    let live = &ed.live;
    if live.fallback || !live.socket_open || !live.connected || !ed.can_edit {
        return;
    }
    let Some(Drag::Dancers { starts, moved: true, .. }) = &ed.drag else {
        ed.live.moving_sent = None;
        return;
    };
    let wait = MOVING_EVERY - (now - live.moving_at);
    if wait > 0.0 {
        app.net.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
        return;
    }
    let f = ed.cur();
    let Some(units) = ed.doc.formations.get(f).map(|x| &x.units) else { return };
    let dancers: Vec<MovingDancer> = starts
        .iter()
        .filter_map(|(i, _)| units.get(*i).map(|u| MovingDancer { index: *i, x: u.position.x, y: u.position.y }))
        .collect();
    if live.moving_sent.as_ref().is_some_and(|(sf, sd)| *sf == f && *sd == dancers) {
        return;
    }
    if ed.live.send(&LiveRequest::Moving { formation: f, dancers: dancers.clone() }) {
        ed.live.moving_sent = Some((f, dancers));
        ed.live.moving_at = now;
    }
}

fn on_sent(app: &mut App, id: ChoreographyId, batch: u64, r: net::ApiResult<Saved>) {
    let Some(ed) = editor_for(app, id) else { return };
    let now = ed.now;
    let live = &mut ed.live;
    live.inflight = None;
    match r {
        Ok(saved) => {
            live.failures = 0;
            live.offline = false;
            if let Some(b) = live.unacked.iter_mut().find(|b| b.id == batch) {
                b.acked_rev = Some(saved.summary.revision);
            }
            let rev = ed.summary.revision;
            ed.summary = saved.summary;
            ed.summary.revision = rev;
            if !ed.live.link_up() {
                // No live stream to confirm it: catch up by reloading.
                resync(app, id);
            }
        }
        Err(ApiError::Unauthorized) => web::navigate("/login"),
        Err(ApiError::Network(_)) | Err(ApiError::Http { status: 500.., .. }) => {
            // Offline or the server is busy: try the same batch again (safe to resend).
            live.failures += 1;
            live.offline = true;
            live.retry_at = now + f64::from(2u32.pow(live.failures.min(4))).min(15.0);
        }
        Err(e) => {
            // Refused (no permission, storage full, broken change): drop it and
            // go back to what the server has.
            live.unacked.retain(|b| b.id != batch);
            app.fail(e);
            resync(app, id);
        }
    }
}

fn open_socket(ed: &mut Editor, app: &mut App, id: ChoreographyId, now: f64) {
    let live = &mut ed.live;
    if live.socket.is_some() || live.socket_closed_at.is_some_and(|t| now - t < RECONNECT_SECS) {
        return;
    }
    let path = format!("/api/choreographies/{id}/ws?session={}", live.session);
    let (net1, net2, net3) = (app.net.clone(), app.net.clone(), app.net.clone());
    live.socket = web::Socket::open(
        &path,
        move || {
            net1.post(move |app: &mut App| {
                if let Some(ed) = editor_for(app, id) {
                    ed.live.socket_open = true;
                    ed.live.socket_ever_opened = true;
                    ed.live.socket_failures = 0;
                    ed.live.offline = false;
                }
            })
        },
        move |text| net2.post(move |app: &mut App| on_event(app, id, &text)),
        move |opened| {
            net3.post(move |app: &mut App| {
                let Some(ed) = editor_for(app, id) else { return };
                let now = ed.now;
                ed.live.socket_closed(now, opened);
                app.net.ctx().request_repaint();
            })
        },
    );
    if live.socket.is_none() {
        live.fallback = true;
    }
}

fn open_stream(ed: &mut Editor, app: &mut App, id: ChoreographyId, now: f64) {
    let live = &mut ed.live;
    if live.stream.is_some() && live.stream_dead_since.is_none() {
        return;
    }
    // A stream the browser gave up on is reopened every few seconds.
    if live.stream_dead_since.is_some_and(|t| now - t < 5.0) {
        return;
    }
    let url = format!("/api/choreographies/{id}/live?session={}", live.session);
    let (net1, net2) = (app.net.clone(), app.net.clone());
    live.stream = web::EventStream::open(
        &url,
        move |text| net1.post(move |app: &mut App| on_event(app, id, &text)),
        move |gave_up| {
            net2.post(move |app: &mut App| {
                if let Some(ed) = editor_for(app, id) {
                    let now = ed.now;
                    if gave_up {
                        ed.live.stream_dead_since = Some(now);
                    }
                    ed.live.viewers.clear();
                    ed.live.connected = false;
                }
            })
        },
    );
    live.stream_dead_since = live.stream.is_none().then_some(now);
}

fn on_event(app: &mut App, id: ChoreographyId, text: &str) {
    let Ok(ev) = serde_json::from_str::<LiveEvent>(text) else { return };
    if matches!(ev, LiveEvent::Deleted) {
        app.close_editor();
        app.toast_error("Someone deleted this choreography.");
        return;
    }
    let Some(ed) = editor_for(app, id) else { return };
    match ev {
        LiveEvent::Saved { batch, summary } => {
            if ed.live.inflight == Some(batch) {
                on_sent(app, id, batch, Ok(Saved { summary }));
            }
            return;
        }
        LiveEvent::Refused { batch, status, error } => {
            if ed.live.inflight == Some(batch) {
                let e = match status {
                    401 => ApiError::Unauthorized,
                    507 => ApiError::StorageFull,
                    _ => ApiError::Http { status, message: error.error, code: error.code },
                };
                on_sent(app, id, batch, Err(e));
            }
            return;
        }
        LiveEvent::Moving { session, formation, dancers } => {
            if session != ed.live.session {
                ed.remote_moving(formation, &dancers);
            }
            return;
        }
        _ => {}
    }
    ed.live.stream_dead_since = None;
    if apply_event(ed, ev) {
        resync(app, id);
    }
}

/// Returns whether the document must be reloaded.
fn apply_event(ed: &mut Editor, ev: LiveEvent) -> bool {
    match ev {
        LiveEvent::Hello { revision } => {
            // A new stream: the server knows nothing of where we are yet.
            ed.live.connected = true;
            ed.live.presence_sent = None;
            return revision != ed.live.server_rev;
        }
        LiveEvent::Presence { viewers } => ed.live.viewers = viewers,
        LiveEvent::Patch { revision, session, batch, by, ops } => {
            if revision <= ed.live.server_rev {
                return false;
            }
            if revision != ed.live.server_rev + 1 {
                return true;
            }
            ed.collect();
            let live = &mut ed.live;
            patch::apply(&mut live.server, &ops);
            let image = patch::take_image(&mut live.server);
            live.server_rev = revision;
            ed.summary.revision = revision;
            ed.summary.updated_by = by;
            if session == ed.live.session {
                // Ours, now stored: the document already has it.
                ed.live.unacked.retain(|b| b.id != batch);
                return false;
            }
            if ops.iter().any(patch::touches_image) && !ed.live.has_image_op() {
                ed.bg_data = image.map(Rc::new);
                ed.live.bg_sent = ed.bg_data.clone();
            }
            ed.live.remote_log.push(Rc::new(ops));
            let f = ed.cur();
            let before = ed.shown_positions(f);
            if !ed.rebuild() {
                return true;
            }
            ed.glide_to_changes(f, &before);
            return false;
        }
        LiveEvent::Summary { summary } => {
            let music_changed = summary.music != ed.summary.music;
            let rev = ed.summary.revision;
            ed.summary = summary;
            ed.summary.revision = rev;
            if music_changed {
                ed.reload_audio();
            }
        }
        LiveEvent::Deleted | LiveEvent::Saved { .. } | LiveEvent::Refused { .. } | LiveEvent::Moving { .. } => {}
    }
    false
}

/// Reload the document from the server and replay our unconfirmed changes on top.
fn resync(app: &mut App, id: ChoreographyId) {
    let net = app.net.clone();
    let Some(ed) = editor_for(app, id) else { return };
    if ed.live.resyncing {
        return;
    }
    ed.live.resyncing = true;
    ed.collect();
    net.spawn(async move {
        let r = net::get::<ChoreographyDoc>(&format!("/api/choreographies/{id}")).await;
        move |app: &mut App| {
            let Some(ed) = editor_for(app, id) else { return };
            ed.live.resyncing = false;
            match r {
                Ok(doc) => {
                    ed.collect();
                    ed.may_write = doc.can_write;
                    let mut v = value_of(&doc.data);
                    let image = patch::take_image(&mut v);
                    let live = &mut ed.live;
                    live.server = v;
                    live.server_rev = doc.summary.revision;
                    let rev = live.server_rev;
                    live.unacked.retain(|b| b.acked_rev.is_none_or(|r| r > rev));
                    if !live.has_image_op() {
                        ed.bg_data = image.map(Rc::new);
                        ed.live.bg_sent = ed.bg_data.clone();
                    }
                    ed.summary = doc.summary;
                    // A full reload: history no longer lines up with other people's changes.
                    ed.live.remote_log.clear();
                    ed.clear_history();
                    if !ed.rebuild() {
                        app.toast_error("Could not apply the latest changes; please reopen the choreography.");
                    }
                }
                Err(ApiError::Unauthorized) => web::navigate("/login"),
                Err(_) => {
                    // Try again shortly (the stream's next hello will ask too).
                    ed.live.server_rev = -2;
                    if ed.live.fallback {
                        ed.live.stream_dead_since.get_or_insert(ed.now);
                    } else if ed.live.socket.is_some() {
                        // Reconnect shortly: the new socket's hello asks again.
                        ed.live.socket_closed(ed.now, true);
                    }
                }
            }
        }
    });
}

fn editor_for(app: &mut App, id: ChoreographyId) -> Option<&mut Editor> {
    app.editor.as_deref_mut().filter(|e| e.summary.id == id)
}
