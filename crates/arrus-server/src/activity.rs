//! What each account does, by day, for the admin's statistics: minutes in
//! Eightcount, logins, edits, choreographies created, exports and music
//! uploads.
//!
//! Counted in memory (no database write on the hot paths) and written about
//! once a minute, before statistics are read, and on shutdown. Rows go with
//! their account and are deleted after [`arrus_api::STATS_MAX_DAYS`] days.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, params};

use crate::{AppState, db};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS activity (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- UTC day number (unix seconds / 86400)
    day         INTEGER NOT NULL,
    -- minutes | logins | edits | created | exports | uploads
    kind        TEXT NOT NULL,
    count       INTEGER NOT NULL,
    PRIMARY KEY (account_id, day, kind)
);
CREATE INDEX IF NOT EXISTS activity_day ON activity(day);
"#;

/// Someone active at all in a slot counts its length in minutes.
pub const SLOT_SECS: i64 = 300;
const FLUSH_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Minutes,
    Logins,
    Edits,
    Created,
    Exports,
    Uploads,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Minutes, Kind::Logins, Kind::Edits, Kind::Created, Kind::Exports, Kind::Uploads];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Minutes => "minutes",
            Kind::Logins => "logins",
            Kind::Edits => "edits",
            Kind::Created => "created",
            Kind::Exports => "exports",
            Kind::Uploads => "uploads",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

pub fn day_of(unix: i64) -> i64 {
    unix.div_euclid(86_400)
}

#[derive(Default)]
pub struct Tracker(Mutex<Pending>);

#[derive(Default)]
struct Pending {
    counts: HashMap<(i64, i64, Kind), i64>,
    /// Each account's last slot counted in minutes.
    last_slot: HashMap<i64, i64>,
}

impl Tracker {
    fn lock(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The account did something: also counts as being active now.
    pub fn add(&self, account: i64, kind: Kind, n: i64) {
        self.add_at(db::now(), account, kind, n);
    }

    fn add_at(&self, now: i64, account: i64, kind: Kind, n: i64) {
        let mut p = self.lock();
        seen_locked(&mut p, now, account);
        if n > 0 {
            *p.counts.entry((account, day_of(now), kind)).or_default() += n;
        }
    }

    /// The account is using Eightcount right now.
    pub fn seen(&self, account: i64) {
        let mut p = self.lock();
        seen_locked(&mut p, db::now(), account);
    }

    pub fn is_empty(&self) -> bool {
        self.lock().counts.is_empty()
    }

    /// Write what was counted (and drop rows past the retention).
    pub fn flush(&self, conn: &Connection) -> rusqlite::Result<()> {
        let counts = std::mem::take(&mut self.lock().counts);
        if counts.is_empty() {
            return Ok(());
        }
        let today = day_of(db::now());
        // Not a transaction: the caller may already be inside one, and each
        // statement is atomic on its own.
        let mut stmt = conn.prepare_cached(
            "INSERT INTO activity (account_id, day, kind, count) \
             SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM accounts WHERE id = ?1) \
             ON CONFLICT(account_id, day, kind) DO UPDATE SET count = count + excluded.count",
        )?;
        for ((account, day, kind), n) in counts {
            stmt.execute(params![account, day, kind.as_str(), n])?;
        }
        conn.execute("DELETE FROM activity WHERE day < ?1", [today - arrus_api::STATS_MAX_DAYS as i64])?;
        Ok(())
    }
}

fn seen_locked(p: &mut Pending, now: i64, account: i64) {
    let slot = now.div_euclid(SLOT_SECS);
    if p.last_slot.insert(account, slot) != Some(slot) {
        *p.counts.entry((account, day_of(now), Kind::Minutes)).or_default() += SLOT_SECS / 60;
    }
}

/// Write the counts about once a minute.
pub fn spawn_flusher(state: &AppState) {
    let weak = std::sync::Arc::downgrade(&state.0);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(FLUSH_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(inner) = weak.upgrade() else { break };
            let state = AppState(inner);
            if state.activity.is_empty() {
                continue;
            }
            let r = state.db_live(|s, conn| Ok(s.activity.flush(conn)?)).await;
            if let Err(e) = r {
                tracing::warn!("writing activity failed: {e:?}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_once_per_slot() {
        let t = Tracker::default();
        let base = 1_000 * SLOT_SECS;
        t.add_at(base, 1, Kind::Edits, 1);
        t.add_at(base + 10, 1, Kind::Edits, 2);
        t.add_at(base + SLOT_SECS, 1, Kind::Edits, 0);
        t.add_at(base + 10, 2, Kind::Logins, 1);
        let p = t.lock();
        let day = day_of(base);
        assert_eq!(p.counts[&(1, day, Kind::Minutes)], 10);
        assert_eq!(p.counts[&(1, day, Kind::Edits)], 3);
        assert_eq!(p.counts[&(2, day, Kind::Minutes)], 5);
        assert_eq!(p.counts[&(2, day, Kind::Logins)], 1);
    }

    #[test]
    fn days_are_utc() {
        assert_eq!(day_of(0), 0);
        assert_eq!(day_of(86_399), 0);
        assert_eq!(day_of(86_400), 1);
        assert_eq!(day_of(-1), -1);
    }
}
