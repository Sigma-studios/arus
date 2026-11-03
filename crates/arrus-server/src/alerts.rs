//! Discord alerts about storage use. The last alerted level is persisted in the
//! `meta` table so a restart does not alert again.

use std::time::Duration;

use rusqlite::Connection;

use crate::db;

const LEVEL_KEY: &str = "alert_level";
const FULL_KEY: &str = "last_full_alert";
/// At most one "storage full" alert per this many seconds.
pub const FULL_ALERT_INTERVAL: i64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Crossed80,
    Crossed95,
    BackUnder75,
    StorageFull,
}

/// The level after observing `used`/`limit`, given the last alerted level
/// (0, 80 or 95), and the alert to send, if any.
pub fn transition(prev: u8, used: u64, limit: u64) -> (u8, Option<AlertKind>) {
    let ratio = if limit == 0 {
        if used > 0 { f64::INFINITY } else { 0.0 }
    } else {
        used as f64 / limit as f64
    };
    if ratio >= 0.95 && prev < 95 {
        (95, Some(AlertKind::Crossed95))
    } else if ratio >= 0.80 && prev < 80 {
        (80, Some(AlertKind::Crossed80))
    } else if ratio < 0.75 && prev > 0 {
        (0, Some(AlertKind::BackUnder75))
    } else {
        (prev, None)
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

pub fn message(kind: AlertKind, used: u64, limit: u64, mention: Option<&str>, public_url: Option<&str>) -> String {
    let pct = if limit == 0 { 100.0 } else { used as f64 * 100.0 / limit as f64 };
    let what = match kind {
        AlertKind::Crossed80 => "storage is over 80 %",
        AlertKind::Crossed95 => "storage is over 95 %, uploads will soon be refused",
        AlertKind::BackUnder75 => "storage is back under 75 %",
        AlertKind::StorageFull => "storage is full: a save or upload was refused",
    };
    let mut s = String::new();
    if let Some(m) = mention.filter(|_| kind != AlertKind::BackUnder75) {
        s.push_str(m);
        s.push(' ');
    }
    s.push_str(&format!("arus: {what} ({} / {} GB, {pct:.0} %)", gb(used), gb(limit)));
    if let Some(url) = public_url {
        s.push_str(&format!(" {url}"));
    }
    s
}

/// Re-evaluate the alert level against current usage; persists the new level and
/// returns the alert to send.
pub fn evaluate(conn: &Connection, used: u64, limit: u64) -> rusqlite::Result<Option<AlertKind>> {
    let prev: u8 = db::meta_get(conn, LEVEL_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    let (level, alert) = transition(prev, used, limit);
    if level != prev {
        db::meta_set(conn, LEVEL_KEY, &level.to_string())?;
    }
    Ok(alert)
}

/// Whether a "storage full" alert is due (at most one per hour); records it if so.
pub fn storage_full_due(conn: &Connection, now: i64) -> rusqlite::Result<bool> {
    let last: i64 = db::meta_get(conn, FULL_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    if now - last >= FULL_ALERT_INTERVAL {
        db::meta_set(conn, FULL_KEY, &now.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Posts alerts to the Discord webhook without ever blocking the caller.
#[derive(Clone)]
pub struct Notifier {
    webhook: Option<String>,
    mention: Option<String>,
    public_url: Option<String>,
    client: reqwest::Client,
    runtime: tokio::runtime::Handle,
}

impl Notifier {
    /// Must be called from within a tokio runtime.
    pub fn new(webhook: Option<String>, mention: Option<String>, public_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { webhook, mention, public_url, client, runtime: tokio::runtime::Handle::current() }
    }

    pub fn send(&self, kind: AlertKind, used: u64, limit: u64) {
        let msg = message(kind, used, limit, self.mention.as_deref(), self.public_url.as_deref());
        let Some(url) = self.webhook.clone() else {
            tracing::info!("storage alert (no ARUS_DISCORD_WEBHOOK): {msg}");
            return;
        };
        tracing::info!("storage alert: {msg}");
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let res = client.post(&url).json(&serde_json::json!({ "content": msg })).send().await;
            match res {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => tracing::error!("discord webhook answered {}", r.status()),
                Err(e) => tracing::error!("discord webhook failed: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: u64 = 1000;

    #[test]
    fn transitions() {
        assert_eq!(transition(0, 500, L), (0, None));
        assert_eq!(transition(0, 800, L), (80, Some(AlertKind::Crossed80)));
        assert_eq!(transition(80, 850, L), (80, None));
        assert_eq!(transition(80, 950, L), (95, Some(AlertKind::Crossed95)));
        // jumping straight past 95 alerts once, at 95
        assert_eq!(transition(0, 990, L), (95, Some(AlertKind::Crossed95)));
        assert_eq!(transition(95, 1000, L), (95, None));
        // falling between 75 and 95 keeps the level: no re-alerts
        assert_eq!(transition(95, 800, L), (95, None));
        assert_eq!(transition(95, 760, L), (95, None));
        assert_eq!(transition(95, 749, L), (0, Some(AlertKind::BackUnder75)));
        assert_eq!(transition(80, 700, L), (0, Some(AlertKind::BackUnder75)));
        assert_eq!(transition(0, 700, L), (0, None));
        assert_eq!(transition(0, 0, 0), (0, None));
        assert_eq!(transition(0, 1, 0), (95, Some(AlertKind::Crossed95)));
    }

    #[test]
    fn evaluate_persists_level() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        assert_eq!(evaluate(&conn, 100, L).unwrap(), None);
        assert_eq!(evaluate(&conn, 810, L).unwrap(), Some(AlertKind::Crossed80));
        // a "restart" reading the same db does not re-alert
        assert_eq!(evaluate(&conn, 820, L).unwrap(), None);
        assert_eq!(evaluate(&conn, 960, L).unwrap(), Some(AlertKind::Crossed95));
        assert_eq!(evaluate(&conn, 960, L).unwrap(), None);
        assert_eq!(evaluate(&conn, 100, L).unwrap(), Some(AlertKind::BackUnder75));
        assert_eq!(evaluate(&conn, 100, L).unwrap(), None);
        assert_eq!(evaluate(&conn, 800, L).unwrap(), Some(AlertKind::Crossed80));
    }

    #[test]
    fn storage_full_rate_limited() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        assert!(storage_full_due(&conn, 10_000).unwrap());
        assert!(!storage_full_due(&conn, 10_100).unwrap());
        assert!(!storage_full_due(&conn, 10_000 + FULL_ALERT_INTERVAL - 1).unwrap());
        assert!(storage_full_due(&conn, 10_000 + FULL_ALERT_INTERVAL).unwrap());
    }

    #[test]
    fn messages() {
        let gib = 1024 * 1024 * 1024;
        let m = message(AlertKind::Crossed80, 4 * gib, 5 * gib, Some("@here"), Some("https://arus.example"));
        assert_eq!(m, "@here arus: storage is over 80 % (4.00 / 5.00 GB, 80 %) https://arus.example");
        let m = message(AlertKind::BackUnder75, gib, 5 * gib, Some("@here"), None);
        assert_eq!(m, "arus: storage is back under 75 % (1.00 / 5.00 GB, 20 %)");
    }
}
