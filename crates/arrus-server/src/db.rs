//! SQLite: schema, connection setup, and shared row helpers.

use std::path::Path;
use std::time::Duration;

use arrus_api::{ChoreographySummary, MusicInfo};
use rusqlite::{Connection, OptionalExtension, Row, params};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS teams (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS members (
    id              INTEGER PRIMARY KEY,
    team_id         INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    username        TEXT NOT NULL,
    username_lower  TEXT NOT NULL UNIQUE,
    password        TEXT NOT NULL,
    can_write       INTEGER NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS members_team ON members(team_id);
-- member_id NULL = the master admin.
CREATE TABLE IF NOT EXISTS sessions (
    token_hash  TEXT PRIMARY KEY,
    member_id   INTEGER REFERENCES members(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_member ON sessions(member_id);
CREATE TABLE IF NOT EXISTS choreographies (
    id                  INTEGER PRIMARY KEY,
    team_id             INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    data                TEXT NOT NULL,
    size_bytes          INTEGER NOT NULL,
    formation_count     INTEGER NOT NULL,
    dancer_count        INTEGER NOT NULL,
    uuid                TEXT NOT NULL,
    creation_timestamp  INTEGER NOT NULL,
    revision            INTEGER NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    updated_by          TEXT NOT NULL,
    -- music: file name under <data>/music/, original name, type, size
    music_file          TEXT,
    music_filename      TEXT,
    music_content_type  TEXT,
    music_bytes         INTEGER
);
CREATE INDEX IF NOT EXISTS choreographies_team ON choreographies(team_id);
CREATE TABLE IF NOT EXISTS meta (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
"#;

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    let _mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn meta_get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Columns read by [`summary_from_row`], in order.
pub const SUMMARY_COLS: &str = "id, team_id, name, revision, updated_at, updated_by, size_bytes, \
     formation_count, dancer_count, music_filename, music_content_type, music_bytes";

pub fn summary_from_row(r: &Row<'_>) -> rusqlite::Result<ChoreographySummary> {
    let music_filename: Option<String> = r.get(9)?;
    let music = match music_filename {
        Some(filename) => Some(MusicInfo {
            filename,
            content_type: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
            bytes: r.get::<_, Option<i64>>(11)?.unwrap_or(0) as u64,
        }),
        None => None,
    };
    Ok(ChoreographySummary {
        id: r.get(0)?,
        team_id: r.get(1)?,
        name: r.get(2)?,
        revision: r.get(3)?,
        updated_at: r.get(4)?,
        updated_by: r.get(5)?,
        size_bytes: r.get::<_, i64>(6)? as u64,
        formation_count: r.get::<_, i64>(7)? as u32,
        dancer_count: r.get::<_, i64>(8)? as u32,
        music,
    })
}

pub fn summary(conn: &Connection, id: i64) -> rusqlite::Result<Option<ChoreographySummary>> {
    conn.query_row(
        &format!("SELECT {SUMMARY_COLS} FROM choreographies WHERE id = ?1"),
        [id],
        summary_from_row,
    )
    .optional()
}

/// What access checks and writes need to know about a stored choreography.
#[derive(Debug, Clone)]
pub struct ChoreoMeta {
    pub id: i64,
    pub team_id: i64,
    pub revision: i64,
    pub size_bytes: u64,
    pub uuid: String,
    pub creation_timestamp: i64,
    pub updated_at: i64,
    pub updated_by: String,
    pub music_file: Option<String>,
    pub music_bytes: u64,
}

pub fn choreo_meta(conn: &Connection, id: i64) -> rusqlite::Result<Option<ChoreoMeta>> {
    conn.query_row(
        "SELECT id, team_id, revision, size_bytes, uuid, creation_timestamp, updated_at, updated_by, \
         music_file, music_bytes FROM choreographies WHERE id = ?1",
        [id],
        |r| {
            Ok(ChoreoMeta {
                id: r.get(0)?,
                team_id: r.get(1)?,
                revision: r.get(2)?,
                size_bytes: r.get::<_, i64>(3)? as u64,
                uuid: r.get(4)?,
                creation_timestamp: r.get(5)?,
                updated_at: r.get(6)?,
                updated_by: r.get(7)?,
                music_file: r.get(8)?,
                music_bytes: r.get::<_, Option<i64>>(9)?.unwrap_or(0) as u64,
            })
        },
    )
    .optional()
}

pub fn team_exists(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM teams WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}
