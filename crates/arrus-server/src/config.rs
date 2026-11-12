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
    /// `ARUS_PUBLIC_URL`, included in alerts and the links in emails.
    pub public_url: Option<String>,
    /// Sending emails (confirm the address, reset a password); `None`: off.
    pub mail: Option<Mail>,
}

/// Where emails come from and how they are sent.
#[derive(Debug, Clone)]
pub struct Mail {
    pub transport: MailTransport,
    /// `ARUS_MAIL_FROM`: the sender address, e.g. `no-reply@example.com`.
    pub from: String,
    /// `ARUS_MAIL_FROM_NAME`, default `Eightcount`.
    pub from_name: String,
    /// `ARUS_MAIL_REPLY_TO`: where answers go (the sender is not read).
    pub reply_to: Option<String>,
}

#[derive(Clone)]
pub enum MailTransport {
    /// Brevo's transactional email API (`ARUS_BREVO_API_KEY`).
    Brevo { api_key: String },
    /// Kept in memory and logged, not sent (tests, local development).
    Memory,
}

impl std::fmt::Debug for MailTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MailTransport::Brevo { .. } => f.write_str("Brevo { api_key: <hidden> }"),
            MailTransport::Memory => f.write_str("Memory"),
        }
    }
}

impl Mail {
    /// In memory, for tests.
    pub fn memory() -> Self {
        Self { transport: MailTransport::Memory, from: "no-reply@example.com".into(), from_name: "Eightcount".into(), reply_to: None }
    }

    /// `ARUS_BREVO_API_KEY` + `ARUS_MAIL_FROM` (+ `ARUS_PUBLIC_URL` for the links).
    fn from_env(public_url: Option<&str>) -> Option<Self> {
        let key = env_opt("ARUS_BREVO_API_KEY");
        let from = env_opt("ARUS_MAIL_FROM");
        let (Some(api_key), Some(from)) = (key.clone(), from.clone()) else {
            if key.is_some() || from.is_some() {
                tracing::warn!("emails are off: set both ARUS_BREVO_API_KEY and ARUS_MAIL_FROM");
            }
            return None;
        };
        if public_url.is_none() {
            tracing::warn!("emails are off: ARUS_PUBLIC_URL is needed for the links in them");
            return None;
        }
        Some(Self {
            transport: MailTransport::Brevo { api_key },
            from,
            from_name: env_opt("ARUS_MAIL_FROM_NAME").unwrap_or_else(|| "Eightcount".into()),
            reply_to: env_opt("ARUS_MAIL_REPLY_TO"),
        })
    }
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
        let public_url = env_opt("ARUS_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string());
        Ok(Self {
            admin_password,
            port: port_from_env()?,
            data_dir: env_opt("ARUS_DATA_DIR").unwrap_or_else(|| "./data".into()).into(),
            storage_limit,
            format_key,
            insecure_cookies,
            dev_autologin,
            mail: Mail::from_env(public_url.as_deref()),
            public_url,
        })
    }
}
