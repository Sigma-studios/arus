//! The Eightcount backend: accounts, teams, choreography storage, and the embedded web client.

pub mod alerts;
pub mod auth;
pub mod config;
pub mod db;
pub mod docs;
pub mod email;
pub mod error;
pub mod healthcheck;
pub mod live;
pub mod mail;
pub mod routes;
pub mod storage;
pub mod web;
pub mod words;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use axum::Router;
use rusqlite::Connection;

pub use config::Config;
use error::{Error, Result};

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub config: Config,
    pub music_dir: PathBuf,
    pub notifier: alerts::Notifier,
    /// `None`: the server sends no emails (see [`Config::mail`]).
    pub mailer: Option<mail::Mailer>,
    /// Who has which choreography open, for live co-editing.
    pub hub: live::Hub,
    db: Mutex<Connection>,
    /// Live changes not written to the database yet (locked under `db`).
    docs: Mutex<docs::Docs>,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    /// Open (or create) the data directory and database. Must run inside a tokio runtime.
    pub fn open(config: Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("creating data dir {}", config.data_dir.display()))?;
        let music_dir = config.data_dir.join("music");
        std::fs::create_dir_all(&music_dir).with_context(|| format!("creating {}", music_dir.display()))?;
        let db_path = config.data_dir.join("arus.db");
        let conn = db::open(&db_path).with_context(|| format!("opening {}", db_path.display()))?;
        conn.execute("DELETE FROM sessions WHERE expires_at < ?1", [db::now()])?;
        // The Discord webhook is set by the admin in the web UI.
        let notifier = alerts::Notifier::new(alerts::Discord::load(&conn)?, config.public_url.clone());
        let mailer = config.mail.clone().map(mail::Mailer::new);
        if mailer.is_none() {
            tracing::info!("emails are off (no ARUS_BREVO_API_KEY / ARUS_MAIL_FROM)");
        }
        let state = AppState(Arc::new(Inner {
            mailer,
            config,
            music_dir,
            notifier,
            hub: live::Hub::default(),
            db: Mutex::new(conn),
            docs: Mutex::default(),
        }));
        {
            // The limit may have changed since the last run.
            let conn = state.db.lock().unwrap_or_else(|p| p.into_inner());
            storage::after_change(&state, &conn)?;
        }
        docs::spawn_flusher(&state);
        Ok(state)
    }

    /// Run `f` with the database connection on the blocking pool, after
    /// writing pending live changes (so `f` sees every choreography as it is).
    pub async fn db<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&AppState, &mut Connection) -> Result<T> + Send + 'static,
    {
        self.run_db(true, f).await
    }

    /// [`AppState::db`] without the flush, for the live hot paths (session
    /// lookup, patches, presence) that never read choreography rows as a whole.
    pub async fn db_live<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&AppState, &mut Connection) -> Result<T> + Send + 'static,
    {
        self.run_db(false, f).await
    }

    async fn run_db<T, F>(&self, flush: bool, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&AppState, &mut Connection) -> Result<T> + Send + 'static,
    {
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = state.db.lock().unwrap_or_else(|p| p.into_inner());
            if flush && state.docs().any_dirty() {
                docs::flush(&state, &conn);
            }
            f(&state, &mut conn)
        })
        .await
        .map_err(Error::internal)?
    }
}

impl Inner {
    pub(crate) fn docs(&self) -> std::sync::MutexGuard<'_, docs::Docs> {
        self.docs.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The whole application router.
pub fn app(state: AppState) -> Router {
    routes::router(state)
}

/// Write live changes still held in memory (on shutdown).
pub async fn flush(state: &AppState) {
    let _ = state.db(|_, _| Ok(())).await;
}

/// Build the router from a config (opens the data dir).
pub fn build(config: Config) -> anyhow::Result<Router> {
    Ok(app(AppState::open(config)?))
}
