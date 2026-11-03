//! Server configuration, read from the environment.

use std::path::PathBuf;

use anyhow::{Context, bail};
use arrus_core::FormatKey;

/// Default app-wide storage limit: 5 GiB.
pub const DEFAULT_STORAGE_LIMIT: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Config {
    /// `ARUS_ADMIN_PASSWORD` (required).
    pub admin_password: String,
    /// `ARUS_PORT`, default 8080.
    pub port: u16,
    /// `ARUS_DATA_DIR`, default `./data`.
    pub data_dir: PathBuf,
    /// `ARUS_STORAGE_LIMIT_BYTES`, default 5 GiB.
    pub storage_limit: u64,
    /// `ARUS_FORMAT_KEY` / `.arrus-key`; `None` disables import/export.
    pub format_key: Option<FormatKey>,
    /// `ARUS_INSECURE_COOKIES=1`: omit `Secure` (local http dev).
    pub insecure_cookies: bool,
    /// `ARUS_DEV_AUTOLOGIN=1` (only honoured with insecure cookies).
    pub dev_autologin: bool,
    /// `ARUS_DISCORD_WEBHOOK`.
    pub discord_webhook: Option<String>,
    /// `ARUS_DISCORD_MENTION`, e.g. `@here`.
    pub discord_mention: Option<String>,
    /// `ARUS_PUBLIC_URL`, included in alerts.
    pub public_url: Option<String>,
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn env_flag(name: &str) -> bool {
    matches!(env_opt(name).as_deref(), Some("1" | "true" | "yes"))
}

/// `ARUS_PORT`, default 8080.
pub fn port_from_env() -> anyhow::Result<u16> {
    match env_opt("ARUS_PORT") {
        Some(p) => p.parse().with_context(|| format!("ARUS_PORT is not a port: {p}")),
        None => Ok(8080),
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let Ok(admin_password) = std::env::var("ARUS_ADMIN_PASSWORD") else {
            bail!("ARUS_ADMIN_PASSWORD must be set (the password of the `admin` account)");
        };
        if admin_password.is_empty() {
            bail!("ARUS_ADMIN_PASSWORD must not be empty");
        }
        let storage_limit = match env_opt("ARUS_STORAGE_LIMIT_BYTES") {
            Some(v) => v.parse().with_context(|| format!("ARUS_STORAGE_LIMIT_BYTES is not a number: {v}"))?,
            None => DEFAULT_STORAGE_LIMIT,
        };
        let format_key = match FormatKey::from_env_or_file() {
            Ok(k) => Some(k),
            Err(e) => {
                tracing::warn!("no ArrangeUs format key ({e}); import/export disabled");
                None
            }
        };
        let insecure_cookies = env_flag("ARUS_INSECURE_COOKIES");
        let mut dev_autologin = env_flag("ARUS_DEV_AUTOLOGIN");
        if dev_autologin && !insecure_cookies {
            tracing::warn!("ARUS_DEV_AUTOLOGIN is ignored unless ARUS_INSECURE_COOKIES=1");
            dev_autologin = false;
        }
        if dev_autologin {
            tracing::warn!("ARUS_DEV_AUTOLOGIN is on: anyone opening / is logged in as admin");
        }
        Ok(Self {
            admin_password,
            port: port_from_env()?,
            data_dir: env_opt("ARUS_DATA_DIR").unwrap_or_else(|| "./data".into()).into(),
            storage_limit,
            format_key,
            insecure_cookies,
            dev_autologin,
            discord_webhook: env_opt("ARUS_DISCORD_WEBHOOK"),
            discord_mention: env_opt("ARUS_DISCORD_MENTION"),
            public_url: env_opt("ARUS_PUBLIC_URL"),
        })
    }
}
