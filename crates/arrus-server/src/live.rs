//! Live co-editing: who has which choreography open, and the stream of
//! changes sent to them (server-sent events).
//!
//! Events of one choreography are sent while the database lock is held (see
//! the patch handler), so every viewer gets them in revision order.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use arrus_api::{LiveEvent, Viewer};
use tokio::sync::broadcast;

/// Events buffered per viewer; a viewer that falls further behind is told to
/// reload the document.
const BUFFER: usize = 256;

#[derive(Default)]
pub struct Hub {
    rooms: Mutex<HashMap<i64, Room>>,
    next_conn: std::sync::atomic::AtomicU64,
}

struct Room {
    tx: broadcast::Sender<Arc<str>>,
    /// One entry per open stream. A browser that reconnects opens a new
    /// stream with the same session before the old one is noticed gone.
    conns: Vec<(u64, Viewer)>,
}

impl Room {
    /// One entry per session.
    fn viewers(&self) -> Vec<Viewer> {
        let mut out: Vec<Viewer> = Vec::new();
        for (_, v) in &self.conns {
            if !out.iter().any(|o| o.session == v.session) {
                out.push(v.clone());
            }
        }
        out
    }
}

fn encode(ev: &LiveEvent) -> Arc<str> {
    serde_json::to_string(ev).unwrap_or_default().into()
}

impl Hub {
    fn rooms(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Room>> {
        self.rooms.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Send `ev` to everyone viewing choreography `id`.
    pub fn send(&self, id: i64, ev: &LiveEvent) {
        if let Some(room) = self.rooms().get(&id) {
            let _ = room.tx.send(encode(ev));
        }
    }

    /// Add a viewer's stream; returns its connection id (for [`Hub::leave`])
    /// and receiver. Everyone (them included) gets the new presence list.
    pub fn join(&self, id: i64, viewer: Viewer) -> (u64, broadcast::Receiver<Arc<str>>) {
        let conn = self.next_conn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut rooms = self.rooms();
        let room = rooms.entry(id).or_insert_with(|| Room { tx: broadcast::channel(BUFFER).0, conns: Vec::new() });
        let rx = room.tx.subscribe();
        room.conns.push((conn, viewer));
        let _ = room.tx.send(encode(&LiveEvent::Presence { viewers: room.viewers() }));
        (conn, rx)
    }

    pub fn leave(&self, id: i64, conn: u64) {
        let mut rooms = self.rooms();
        let Some(room) = rooms.get_mut(&id) else { return };
        let before = room.viewers();
        room.conns.retain(|(c, _)| *c != conn);
        if room.conns.is_empty() {
            rooms.remove(&id);
        } else if room.viewers() != before {
            let _ = room.tx.send(encode(&LiveEvent::Presence { viewers: room.viewers() }));
        }
    }

    pub fn viewers(&self, id: i64) -> Vec<Viewer> {
        self.rooms().get(&id).map(Room::viewers).unwrap_or_default()
    }
}

/// Removes its stream when it ends (the browser left).
pub struct Leave {
    pub state: crate::AppState,
    pub id: i64,
    pub conn: u64,
}

impl Drop for Leave {
    fn drop(&mut self) {
        self.state.hub.leave(self.id, self.conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(s: &str) -> Viewer {
        Viewer { session: s.into(), username: format!("user-{s}"), can_write: true }
    }

    #[tokio::test]
    async fn presence_and_events() {
        let hub = Hub::default();
        let (a_conn, mut a) = hub.join(1, viewer("a"));
        let first: LiveEvent = serde_json::from_str(&a.recv().await.unwrap()).unwrap();
        assert_eq!(first, LiveEvent::Presence { viewers: vec![viewer("a")] });
        let (b_conn, mut b) = hub.join(1, viewer("b"));
        for rx in [&mut a, &mut b] {
            let ev: LiveEvent = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            assert_eq!(ev, LiveEvent::Presence { viewers: vec![viewer("a"), viewer("b")] });
        }
        hub.send(1, &LiveEvent::Deleted);
        hub.send(2, &LiveEvent::Deleted); // nobody there: dropped
        assert_eq!(serde_json::from_str::<LiveEvent>(&b.recv().await.unwrap()).unwrap(), LiveEvent::Deleted);
        // b reconnects before its old stream is noticed gone: still listed once,
        // and still listed after the old stream ends.
        let (b2_conn, _b2) = hub.join(1, viewer("b"));
        assert_eq!(hub.viewers(1), vec![viewer("a"), viewer("b")]);
        hub.leave(1, b_conn);
        assert_eq!(hub.viewers(1), vec![viewer("a"), viewer("b")]);
        hub.leave(1, a_conn);
        assert_eq!(hub.viewers(1), vec![viewer("b")]);
        hub.leave(1, b2_conn);
        assert!(hub.viewers(1).is_empty());
    }
}
