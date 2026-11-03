//! App-wide storage accounting: choreography JSON + music files.

use rusqlite::Connection;

use crate::alerts::{self, AlertKind};
use crate::error::{Error, Result};
use crate::{AppState, db};

/// Bytes currently used by all teams.
pub fn usage(conn: &Connection) -> rusqlite::Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT COALESCE(SUM(size_bytes), 0) + COALESCE(SUM(music_bytes), 0) FROM choreographies",
        [],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as u64)
}

/// Check that replacing `old` stored bytes with `new` ones fits in the limit.
/// Writes that do not grow usage are always allowed. Call with the connection
/// held for the whole check-and-write so concurrent writes cannot both pass.
pub fn ensure_fits(state: &AppState, conn: &Connection, old: u64, new: u64) -> Result<()> {
    if new <= old {
        return Ok(());
    }
    let used = usage(conn)?;
    let after = used.saturating_sub(old).saturating_add(new);
    let limit = state.config.storage_limit;
    if after > limit {
        if alerts::storage_full_due(conn, db::now())? {
            state.notifier.send(AlertKind::StorageFull, used, limit);
        }
        return Err(Error::storage_full());
    }
    Ok(())
}

/// Re-evaluate alert levels after usage changed.
pub fn after_change(state: &AppState, conn: &Connection) -> rusqlite::Result<()> {
    let used = usage(conn)?;
    let limit = state.config.storage_limit;
    if let Some(kind) = alerts::evaluate(conn, used, limit)? {
        state.notifier.send(kind, used, limit);
    }
    Ok(())
}

/// Remove music files, logging (not failing on) errors. Missing files are fine.
pub fn remove_music_files(state: &AppState, files: impl IntoIterator<Item = String>) {
    for f in files {
        let path = state.music_dir.join(&f);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::error!("could not delete {}: {e}", path.display());
        }
    }
}
