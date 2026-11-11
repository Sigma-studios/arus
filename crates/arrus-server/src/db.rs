//! SQLite: schema, connection setup, and shared row helpers.

use std::path::Path;
use std::time::Duration;

use arrus_api::{ChoreographySummary, MusicInfo};
use rusqlite::{Connection, OptionalExtension, Row, params};

const ACCOUNTS: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id                INTEGER PRIMARY KEY,
    nickname          TEXT NOT NULL,
    nickname_lower    TEXT NOT NULL UNIQUE,
    email             TEXT,
    email_lower       TEXT UNIQUE,
    -- argon2 (PHC string)
    password_hash     TEXT NOT NULL,
    -- the password in clear while it is generated or set by the admin
    -- (NULL once the person chose their own)
    password_visible  TEXT,
    is_admin          INTEGER NOT NULL DEFAULT 0,
    is_premium        INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    -- unix seconds of the last nickname change the person made themselves
    nickname_changed_at INTEGER,
    -- their settings, a JSON UserPrefs
    prefs             TEXT
);
CREATE TABLE IF NOT EXISTS team_members (
    team_id     INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- owner | co_owner | guest
    role        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (team_id, account_id)
);
"#;

const SESSIONS: &str = r#"
-- account_id NULL = the master admin.
CREATE TABLE IF NOT EXISTS sessions (
    token_hash  TEXT PRIMARY KEY,
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);
"#;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS teams (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    -- NULL: made by the admin (no limits)
    owner_id    INTEGER REFERENCES accounts(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS team_members_account ON team_members(account_id);
CREATE INDEX IF NOT EXISTS sessions_account ON sessions(account_id);
-- Links sent by email (see email.rs); the token itself is never stored.
CREATE TABLE IF NOT EXISTS email_tokens (
    token_hash   TEXT PRIMARY KEY,
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- confirm | reset
    kind         TEXT NOT NULL,
    -- the address it was sent to: it only works while the account has it
    email_lower  TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS email_tokens_account ON email_tokens(account_id, kind);
CREATE TABLE IF NOT EXISTS team_invites (
    token       TEXT PRIMARY KEY,
    team_id     INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS team_invites_team ON team_invites(team_id);
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
    let mut conn = Connection::open(path)?;
    init(&mut conn)?;
    Ok(conn)
}

pub fn init(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    let _mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // Off while tables are rebuilt.
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    if table_exists(conn, "members")? {
        members_to_accounts(conn)?;
    }
    conn.execute_batch(ACCOUNTS)?;
    conn.execute_batch(SESSIONS)?;
    conn.execute_batch(SCHEMA)?;
    add_column(conn, "accounts", "nickname_changed_at", "INTEGER")?;
    add_column(conn, "accounts", "prefs", "TEXT")?;
    // unix seconds when the current email was confirmed (NULL: not yet)
    add_column(conn, "accounts", "email_verified_at", "INTEGER")?;
    conn.pragma_update(None, "foreign_keys", "ON")
}

/// Add a column that databases made before it existed lack.
fn add_column(conn: &Connection, table: &str, column: &str, decl: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?;
    if !stmt.exists([column])? {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))?;
    }
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Before accounts, each login was a member of exactly one team, made by the
/// admin, with a generated password. Every member becomes an account (same id,
/// nickname and password, still visible to the admin) in that team: co-owner
/// if they could write, guest otherwise. Sessions survive.
fn members_to_accounts(conn: &mut Connection) -> rusqlite::Result<()> {
    let members: Vec<(i64, i64, String, String, bool, i64)> = {
        let mut stmt = conn.prepare("SELECT id, team_id, username, password, can_write, created_at FROM members")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let tx = conn.transaction()?;
    tx.execute_batch(ACCOUNTS)?;
    tx.execute_batch("ALTER TABLE teams ADD COLUMN owner_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL;")?;
    for (id, team_id, username, password, can_write, created_at) in &members {
        tx.execute(
            "INSERT INTO accounts (id, nickname, nickname_lower, password_hash, password_visible, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, username, username.to_lowercase(), crate::auth::hash_password(password), password, created_at],
        )?;
        let role = if *can_write { arrus_api::TeamRole::CoOwner } else { arrus_api::TeamRole::Guest };
        tx.execute(
            "INSERT INTO team_members (team_id, account_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![team_id, id, role.as_str(), created_at],
        )?;
    }
    tx.execute_batch(
        "CREATE TABLE sessions_new (
             token_hash  TEXT PRIMARY KEY,
             account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
             created_at  INTEGER NOT NULL,
             expires_at  INTEGER NOT NULL
         );
         INSERT INTO sessions_new SELECT token_hash, member_id, created_at, expires_at FROM sessions;
         DROP TABLE sessions;
         ALTER TABLE sessions_new RENAME TO sessions;
         DROP TABLE members;",
    )?;
    tx.commit()?;
    tracing::info!("moved {} team members to accounts", members.len());
    Ok(())
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

/// How many choreographies the team may hold: `None` = unlimited (a team
/// made by the admin, or owned by a premium or admin account).
pub fn choreography_limit(conn: &Connection, team_id: i64) -> rusqlite::Result<Option<u32>> {
    let owner: Option<(bool, bool)> = conn
        .query_row(
            "SELECT a.is_premium, a.is_admin FROM teams t JOIN accounts a ON a.id = t.owner_id WHERE t.id = ?1",
            [team_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match owner {
        Some((false, false)) => Some(arrus_api::FREE_CHOREOGRAPHIES),
        _ => None,
    })
}

pub fn choreography_count(conn: &Connection, team_id: i64) -> rusqlite::Result<u32> {
    conn.query_row("SELECT COUNT(*) FROM choreographies WHERE team_id = ?1", [team_id], |r| r.get::<_, i64>(0))
        .map(|n| n as u32)
}

/// Fail with 403 `limit_choreographies` if `adding` more would pass the team's limit.
pub fn ensure_choreography_room(conn: &Connection, team_id: i64, adding: u32) -> crate::error::Result<()> {
    if let Some(limit) = choreography_limit(conn, team_id)?
        && choreography_count(conn, team_id)? + adding > limit
    {
        return Err(crate::error::Error::with_code(
            axum::http::StatusCode::FORBIDDEN,
            "limit_choreographies",
            format!("a team of a free account holds at most {limit} choreographies (premium: unlimited)"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_SCHEMA: &str = r#"
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

    #[test]
    fn members_become_accounts() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(OLD_SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO teams (id, name, created_at) VALUES (1, 'Unicup', 10);
             INSERT INTO members (id, team_id, username, username_lower, password, can_write, created_at)
                 VALUES (3, 1, 'Mathis', 'mathis', 'tiger-maple-42', 1, 11),
                        (4, 1, 'Lea', 'lea', 'fox-owl-10', 0, 12);
             INSERT INTO sessions (token_hash, member_id, created_at, expires_at) VALUES ('t3', 3, 1, 9999999999),
                                                                                       ('ta', NULL, 1, 9999999999);
             INSERT INTO choreographies (id, team_id, name, data, size_bytes, formation_count, dancer_count, uuid,
                 creation_timestamp, revision, created_at, updated_at, updated_by)
                 VALUES (7, 1, 'Kugel', '{}', 2, 0, 0, 'u', 0, 1, 0, 0, 'Mathis');",
        )
        .unwrap();

        init(&mut conn).unwrap();
        init(&mut conn).unwrap(); // idempotent

        let accounts: Vec<(i64, String, Option<String>)> = conn
            .prepare("SELECT id, nickname, password_visible FROM accounts ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            accounts,
            vec![(3, "Mathis".into(), Some("tiger-maple-42".into())), (4, "Lea".into(), Some("fox-owl-10".into()))]
        );
        let roles: Vec<(i64, String)> = conn
            .prepare("SELECT account_id, role FROM team_members WHERE team_id = 1 ORDER BY account_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(roles, vec![(3, "co_owner".into()), (4, "guest".into())]);
        // same passwords, and both sessions survive
        assert_eq!(crate::auth::check_login(&conn, "x", "mathis", "Tiger-maple-42").unwrap(), Some(Some(3)));
        let a = crate::auth::lookup(&conn, "unused").unwrap();
        assert!(a.is_none());
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        // the team stays the admin's (no owner, no limit), with its choreography
        assert_eq!(choreography_limit(&conn, 1).unwrap(), None);
        assert_eq!(choreography_count(&conn, 1).unwrap(), 1);
        assert!(!table_exists(&conn, "members").unwrap());
        let broken: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(broken, 0);
        // cascades point at accounts now
        conn.execute("DELETE FROM accounts WHERE id = 3", []).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }
}
