//! Write-behind cache of the choreographies being co-edited.
//!
//! Live patches are applied to a copy kept in memory and written to SQLite
//! at most every [`FLUSH_EVERY`] (and when the server stops), instead of a
//! read-parse-write of the whole document per patch. Revisions still count
//! every patch, in memory.
//!
//! Everything here runs under the database connection lock (inside
//! [`AppState::db`] closures), which keeps patches in order. Every database
//! use except the live hot paths ([`AppState::db_live`]) first flushes what
//! is pending, so the rest of the server always reads up-to-date rows. A row
//! changed by anything else (a whole-document save) bumps the revision in
//! the database, which no longer matches the cached one: the cache reloads.

use std::collections::HashMap;
use std::time::Duration;

use arrus_api::{ChoreographySummary, PatchOp, patch as jpatch};
use arrus_core::Choreography;
use rusqlite::{Connection, params};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::routes::Stored;
use crate::{AppState, db, storage};

/// How long a live change may stay in memory only.
pub const FLUSH_EVERY: Duration = Duration::from_secs(2);

struct Doc {
    /// The document as patched so far (identity fields as sent).
    value: Value,
    /// What the database row gets on the next flush.
    stored: Stored,
    /// Revision of the row in the database.
    db_rev: i64,
    /// Revision including the patches not written yet.
    rev: i64,
    updated_at: i64,
    updated_by: String,
    dirty: bool,
}

#[derive(Default)]
pub struct Docs {
    map: HashMap<i64, Doc>,
}

impl Docs {
    pub fn any_dirty(&self) -> bool {
        self.map.values().any(|d| d.dirty)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Apply a batch of live changes to choreography `meta.id` (permission
/// already checked). Returns the summary as of the new revision.
pub fn apply_patch(s: &AppState, conn: &Connection, meta: &db::ChoreoMeta, ops: &[PatchOp], by: &str) -> Result<ChoreographySummary> {
    let id = meta.id;
    let mut docs = s.docs();
    let fresh = docs.map.get(&id).is_some_and(|d| d.db_rev == meta.revision);
    if !fresh {
        let json: String = conn.query_row("SELECT data FROM choreographies WHERE id = ?1", [id], |r| r.get(0))?;
        let value: Value = serde_json::from_str(&json).map_err(Error::internal)?;
        let data: Choreography = serde_json::from_value(value.clone()).map_err(Error::internal)?;
        let stored = Stored::new(&data)?;
        docs.map.insert(
            id,
            Doc {
                value,
                stored,
                db_rev: meta.revision,
                rev: meta.revision,
                updated_at: meta.updated_at,
                updated_by: meta.updated_by.clone(),
                dirty: false,
            },
        );
    }
    let doc = docs.map.get_mut(&id).expect("just inserted");
    let mut value = doc.value.clone();
    jpatch::apply(&mut value, ops);
    let mut data: Choreography = serde_json::from_value(value.clone())
        .map_err(|e| Error::bad_request(format!("this change would break the choreography: {e}")))?;
    let now = db::now();
    // The app's identity of the choreography never changes on save.
    data.id = meta.uuid.clone();
    data.creation_timestamp = meta.creation_timestamp;
    data.last_edit_timestamp = now;
    let stored = Stored::new(&data)?;
    storage::ensure_fits(s, conn, doc.stored.size(), stored.size())?;
    doc.value = value;
    doc.stored = stored;
    doc.rev += 1;
    doc.updated_at = now;
    doc.updated_by = by.to_string();
    doc.dirty = true;

    drop(docs);
    summary(s, conn, id)
}

/// The summary of choreography `id`, counting live changes not written yet.
pub fn summary(s: &AppState, conn: &Connection, id: i64) -> Result<ChoreographySummary> {
    let mut summary = db::summary(conn, id)?.ok_or_else(Error::not_found)?;
    if let Some(d) = s.docs().map.get(&id).filter(|d| d.db_rev == summary.revision) {
        summary.name = d.stored.name.clone();
        summary.revision = d.rev;
        summary.updated_at = d.updated_at;
        summary.updated_by = d.updated_by.clone();
        summary.size_bytes = d.stored.size();
        summary.formation_count = d.stored.formation_count as u32;
        summary.dancer_count = d.stored.dancer_count as u32;
    }
    Ok(summary)
}

/// Write every pending change to the database, then forget documents
/// nobody has open any more.
pub fn flush(s: &AppState, conn: &Connection) {
    let mut docs = s.docs();
    let mut wrote = false;
    for (id, d) in docs.map.iter_mut().filter(|(_, d)| d.dirty) {
        let st = &d.stored;
        let r = conn.execute(
            "UPDATE choreographies SET name = ?1, data = ?2, size_bytes = ?3, formation_count = ?4, \
             dancer_count = ?5, revision = ?6, updated_at = ?7, updated_by = ?8 WHERE id = ?9 AND revision = ?10",
            params![st.name, st.json, st.size() as i64, st.formation_count, st.dancer_count, d.rev, d.updated_at, d.updated_by, id, d.db_rev],
        );
        match r {
            Ok(n) => {
                if n == 0 {
                    // Deleted (or, which cannot happen, changed behind our back).
                    tracing::warn!("choreography {id}: live changes not written, the row is gone or changed");
                }
                d.db_rev = d.rev;
                d.dirty = false;
                wrote = true;
            }
            // Kept for the next flush.
            Err(e) => tracing::error!("choreography {id}: writing live changes failed: {e}"),
        }
    }
    docs.map.retain(|id, d| d.dirty || !s.hub.viewers(*id).is_empty());
    drop(docs);
    if wrote && let Err(e) = storage::after_change(s, conn) {
        tracing::error!("storage alerts: {e}");
    }
}

/// Flush every [`FLUSH_EVERY`] for as long as the server runs.
pub fn spawn_flusher(state: &AppState) {
    let weak = std::sync::Arc::downgrade(&state.0);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(FLUSH_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(inner) = weak.upgrade() else { break };
            let state = AppState(inner);
            if state.docs().is_empty() {
                continue;
            }
            // Also forgets the documents nobody has open any more.
            let _ = state
                .db_live(|s, conn| {
                    flush(s, conn);
                    Ok(())
                })
                .await;
        }
    });
}
