//! Discord alerts about storage use. The last alerted level is persisted in the
//! `meta` table so a restart does not alert again.

use std::sync::{Arc, RwLock};
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

const WEBHOOK_KEY: &str = "discord_webhook";
const MENTION_KEY: &str = "discord_mention";

/// Where alerts go, set by the admin in the web UI and kept in the `meta` table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discord {
    pub webhook: Option<String>,
    /// Already in Discord's syntax (`@here`, `<@123>`, `<@&123>`).
    pub mention: Option<String>,
}

impl Discord {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let get = |k| db::meta_get(conn, k).map(|v| v.filter(|s| !s.is_empty()));
        Ok(Self { webhook: get(WEBHOOK_KEY)?, mention: get(MENTION_KEY)? })
    }

    pub fn save(&self, conn: &Connection) -> rusqlite::Result<()> {
        db::meta_set(conn, WEBHOOK_KEY, self.webhook.as_deref().unwrap_or(""))?;
        db::meta_set(conn, MENTION_KEY, self.mention.as_deref().unwrap_or(""))
    }

    pub fn settings(&self) -> arrus_api::AlertSettings {
        arrus_api::AlertSettings {
            webhook_set: self.webhook.is_some(),
            webhook_hint: self.webhook.as_deref().map(webhook_hint),
            mention: self.mention.clone().unwrap_or_default(),
        }
    }
}

const DISCORD_HOSTS: &[&str] = &[
    "discord.com",
    "discordapp.com",
    "ptb.discord.com",
    "canary.discord.com",
    "ptb.discordapp.com",
    "canary.discordapp.com",
];

/// A Discord webhook URL, trimmed; `None` if it is anything else (the server only
/// ever posts to Discord).
pub fn parse_webhook(input: &str) -> Option<String> {
    let input = input.trim();
    let url = reqwest::Url::parse(input).ok()?;
    let ok = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|h| DISCORD_HOSTS.contains(&h))
        && url.path().starts_with("/api/webhooks/")
        && url.path().split('/').filter(|p| !p.is_empty()).count() >= 4;
    ok.then(|| input.to_owned())
}

/// `discord.com/api/webhooks/1234…/…wxyz`: enough to recognise a webhook, not to use it.
pub fn webhook_hint(url: &str) -> String {
    let Ok(u) = reqwest::Url::parse(url) else { return "set".into() };
    let parts: Vec<&str> = u.path().split('/').filter(|p| !p.is_empty()).collect();
    let id = parts.get(2).copied().unwrap_or("");
    let token = parts.get(3).copied().unwrap_or("");
    let id_start: String = id.chars().take(4).collect();
    let tail: String = token.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{}/api/webhooks/{id_start}…/…{tail}", u.host_str().unwrap_or(""))
}

/// Normalise who alerts tag: `@here`, `@everyone`, `user:<id>`, `role:<id>`, `<@id>`,
/// `<@!id>` or `<@&id>`, separated by spaces or commas. `Some("")` = nobody;
/// `None` = not understood.
pub fn parse_mention(input: &str) -> Option<String> {
    let mut out = Vec::new();
    for t in input.split([' ', ',']).filter(|t| !t.is_empty()) {
        let digits = |s: &str| !s.is_empty() && s.len() <= 25 && s.bytes().all(|b| b.is_ascii_digit());
        let m = if t == "@here" || t == "@everyone" {
            t.to_owned()
        } else if let Some(id) = t.strip_prefix("user:").filter(|s| digits(s)) {
            format!("<@{id}>")
        } else if let Some(id) = t.strip_prefix("role:").filter(|s| digits(s)) {
            format!("<@&{id}>")
        } else if let Some(inner) = t.strip_prefix("<@").and_then(|r| r.strip_suffix('>')) {
            if let Some(id) = inner.strip_prefix('&').filter(|s| digits(s)) {
                format!("<@&{id}>")
            } else if let Some(id) = inner.strip_prefix('!').or(Some(inner)).filter(|s| digits(s)) {
                format!("<@{id}>")
            } else {
                return None;
            }
        } else {
            return None;
        };
        if !out.contains(&m) {
            out.push(m);
        }
    }
    Some(out.join(" "))
}

/// Posts alerts to the Discord webhook the admin configured, without ever blocking
/// the caller.
#[derive(Clone)]
pub struct Notifier {
    discord: Arc<RwLock<Discord>>,
    public_url: Option<String>,
    client: reqwest::Client,
    runtime: tokio::runtime::Handle,
}

impl Notifier {
    /// Must be called from within a tokio runtime.
    pub fn new(discord: Discord, public_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            discord: Arc::new(RwLock::new(discord)),
            public_url,
            client,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    pub fn discord(&self) -> Discord {
        self.discord.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_discord(&self, d: Discord) {
        *self.discord.write().unwrap_or_else(|p| p.into_inner()) = d;
    }

    pub fn send(&self, kind: AlertKind, used: u64, limit: u64) {
        let d = self.discord();
        let msg = message(kind, used, limit, d.mention.as_deref(), self.public_url.as_deref());
        let Some(url) = d.webhook else {
            tracing::info!("storage alert (no Discord webhook set): {msg}");
            return;
        };
        tracing::info!("storage alert: {msg}");
        let me = self.clone();
        self.runtime.spawn(async move {
            if let Err(e) = me.post(&url, &msg).await {
                tracing::error!("{e}");
            }
        });
    }

    /// A test message (tagging nobody), awaited so the admin sees the outcome.
    pub async fn test(&self, used: u64, limit: u64) -> Result<(), String> {
        let url = self.discord().webhook.ok_or_else(|| "no Discord webhook is set".to_owned())?;
        let pct = if limit == 0 { 100.0 } else { used as f64 * 100.0 / limit as f64 };
        let mut msg = format!(
            "arus: test message. Storage alerts will be posted here (now {} / {} GB, {pct:.0} %).",
            gb(used),
            gb(limit)
        );
        if let Some(u) = &self.public_url {
            msg.push_str(&format!(" {u}"));
        }
        self.post(&url, &msg).await
    }

    async fn post(&self, url: &str, content: &str) -> Result<(), String> {
        let res = self.client.post(url).json(&serde_json::json!({ "content": content })).send().await;
        match res {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => {
                let status = r.status();
                let body: String = r.text().await.unwrap_or_default().chars().take(200).collect();
                Err(format!("Discord answered {status}: {body}"))
            }
            Err(e) => Err(format!("could not reach Discord: {e}")),
        }
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

    #[test]
    fn webhooks() {
        let ok = "https://discord.com/api/webhooks/123456789/abcDEF-ghi_jkl";
        assert_eq!(parse_webhook(&format!("  {ok} ")).as_deref(), Some(ok));
        assert!(parse_webhook("https://canary.discord.com/api/webhooks/1/x").is_some());
        for bad in [
            "http://discord.com/api/webhooks/1/x",
            "https://evil.com/api/webhooks/1/x",
            "https://discord.com.evil.com/api/webhooks/1/x",
            "https://discord.com/api/webhooks/1",
            "https://discord.com/api/channels/1/x",
            "https://user:pw@discord.com/api/webhooks/1/x",
            "https://discord.com:8443/api/webhooks/1/x",
            "not a url",
            "",
        ] {
            assert_eq!(parse_webhook(bad), None, "{bad}");
        }
        assert_eq!(webhook_hint(ok), "discord.com/api/webhooks/1234…/…_jkl");
    }

    #[test]
    fn mentions() {
        assert_eq!(parse_mention("").as_deref(), Some(""));
        assert_eq!(parse_mention("@here").as_deref(), Some("@here"));
        assert_eq!(parse_mention("user:42, role:7 @here").as_deref(), Some("<@42> <@&7> @here"));
        assert_eq!(parse_mention("<@!42> <@42> <@&7>").as_deref(), Some("<@42> <@&7>"));
        assert_eq!(parse_mention("@someone"), None);
        assert_eq!(parse_mention("user:abc"), None);
        assert_eq!(parse_mention("<@abc>"), None);
    }

    #[test]
    fn discord_settings_persist() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        assert_eq!(Discord::load(&conn).unwrap(), Discord::default());
        let d = Discord { webhook: Some("https://discord.com/api/webhooks/1/abcd".into()), mention: Some("@here".into()) };
        d.save(&conn).unwrap();
        assert_eq!(Discord::load(&conn).unwrap(), d);
        Discord::default().save(&conn).unwrap();
        assert_eq!(Discord::load(&conn).unwrap(), Discord::default());
    }
}
