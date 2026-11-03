//! The arus backend: accounts, teams, choreography storage, and the embedded web client.

pub mod alerts;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod healthcheck;
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
    db: Mutex<Connection>,
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
        let notifier = alerts::Notifier::new(
            config.discord_webhook.clone(),
            config.discord_mention.clone(),
            config.public_url.clone(),
        );
        let state = AppState(Arc::new(Inner { config, music_dir, notifier, db: Mutex::new(conn) }));
        {
            // The limit may have changed since the last run.
            let conn = state.db.lock().unwrap_or_else(|p| p.into_inner());
            storage::after_change(&state, &conn)?;
        }
        Ok(state)
    }

    /// Run `f` with the database connection on the blocking pool.
    pub async fn db<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&AppState, &mut Connection) -> Result<T> + Send + 'static,
    {
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = state.db.lock().unwrap_or_else(|p| p.into_inner());
            f(&state, &mut conn)
        })
        .await
        .map_err(Error::internal)?
    }
}

/// The whole application router.
pub fn app(state: AppState) -> Router {
    routes::router(state)
}

/// Build the router from a config (opens the data dir).
pub fn build(config: Config) -> anyhow::Result<Router> {
    Ok(app(AppState::open(config)?))
}
