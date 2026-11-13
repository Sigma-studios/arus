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
    conn.execute_batch(crate::activity::SCHEMA)?;
    add_column(conn, "accounts", "nickname_changed_at", "INTEGER")?;
    add_column(conn, "accounts", "prefs", "TEXT")?;
    // unix seconds when the current email was confirmed (NULL: not yet)
    add_column(conn, "accounts", "email_verified_at", "INTEGER")?;
    // unix seconds when the person accepted the terms of use at sign-up
    add_column(conn, "accounts", "terms_accepted_at", "INTEGER")?;
    // Premium ended over the free limits: must pick what stays editable (see [`sync_plan`])
    add_column(conn, "accounts", "must_choose", "INTEGER NOT NULL DEFAULT 0")?;
    // read-only: not kept when the owner's Premium ended
    add_column(conn, "teams", "locked", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(conn, "choreographies", "locked", "INTEGER NOT NULL DEFAULT 0")?;
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
     formation_count, dancer_count, music_filename, music_content_type, music_bytes, \
     (locked OR COALESCE((SELECT l.locked FROM teams l WHERE l.id = choreographies.team_id), 0))";

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
        read_only: r.get(12)?,
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
    /// Read-only: it or its team was not kept when Premium ended.
    pub locked: bool,
}

pub fn choreo_meta(conn: &Connection, id: i64) -> rusqlite::Result<Option<ChoreoMeta>> {
    conn.query_row(
        "SELECT c.id, c.team_id, c.revision, c.size_bytes, c.uuid, c.creation_timestamp, c.updated_at, c.updated_by, \
         c.music_file, c.music_bytes, (c.locked OR t.locked) \
         FROM choreographies c JOIN teams t ON t.id = c.team_id WHERE c.id = ?1",
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
                locked: r.get(10)?,
            })
        },
    )
    .optional()
}

pub fn team_exists(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM teams WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

/// The plan a team falls under: its owner's (`None`: no owner, made by the
/// master admin: unlimited), and the owner's id.
pub fn team_plan(conn: &Connection, team_id: i64) -> rusqlite::Result<(Option<i64>, arrus_api::PlanLimits)> {
    let owner: Option<(i64, bool, bool)> = conn
        .query_row(
            "SELECT a.id, a.is_premium, a.is_admin FROM teams t JOIN accounts a ON a.id = t.owner_id WHERE t.id = ?1",
            [team_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(match owner {
        Some((id, premium, admin)) => (Some(id), arrus_api::PlanLimits::of(premium, admin)),
        None => (None, arrus_api::PlanLimits::UNLIMITED),
    })
}

/// How many choreographies the team may hold: `None` = unlimited.
pub fn choreography_limit(conn: &Connection, team_id: i64) -> rusqlite::Result<Option<u32>> {
    Ok(team_plan(conn, team_id)?.1.choreographies_per_team)
}

/// Music in all the teams `owner` owns.
pub fn music_used(conn: &Connection, owner: i64) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COALESCE(SUM(c.music_bytes), 0) FROM choreographies c JOIN teams t ON t.id = c.team_id WHERE t.owner_id = ?1",
        [owner],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as u64)
}

/// Fail with 403 `limit_music` (the plan has no music) or `limit_music_storage`
/// if a track of `adding` bytes, replacing one of `replacing`, does not fit.
pub fn ensure_music_room(conn: &Connection, team_id: i64, replacing: u64, adding: u64) -> crate::error::Result<()> {
    let (owner, plan) = team_plan(conn, team_id)?;
    let (Some(owner), Some(limit)) = (owner, plan.music_bytes) else { return Ok(()) };
    if limit == 0 {
        return Err(crate::error::Error::with_code(
            axum::http::StatusCode::FORBIDDEN,
            "limit_music",
            "music is a premium feature: this team is on the free plan",
        ));
    }
    let used = music_used(conn, owner)?.saturating_sub(replacing);
    if used + adding > limit {
        return Err(crate::error::Error::with_code(
            axum::http::StatusCode::FORBIDDEN,
            "limit_music_storage",
            format!("the music storage of this plan is full ({} MB)", limit / (1024 * 1024)),
        ));
    }
    Ok(())
}

pub fn choreography_count(conn: &Connection, team_id: i64) -> rusqlite::Result<u32> {
    conn.query_row("SELECT COUNT(*) FROM choreographies WHERE team_id = ?1", [team_id], |r| r.get::<_, i64>(0))
        .map(|n| n as u32)
}

/// Choreographies that count toward the team's limit: the editable ones.
pub fn editable_choreography_count(conn: &Connection, team_id: i64) -> rusqlite::Result<u32> {
    conn.query_row("SELECT COUNT(*) FROM choreographies WHERE team_id = ?1 AND NOT locked", [team_id], |r| {
        r.get::<_, i64>(0)
    })
    .map(|n| n as u32)
}

pub fn team_locked(conn: &Connection, team_id: i64) -> rusqlite::Result<bool> {
    Ok(conn.query_row("SELECT locked FROM teams WHERE id = ?1", [team_id], |r| r.get(0)).optional()?.unwrap_or(false))
}

pub fn read_only_error() -> crate::error::Error {
    crate::error::Error::with_code(
        axum::http::StatusCode::FORBIDDEN,
        "read_only",
        "this is read-only since Premium ended: it can be viewed, exported and deleted, not changed",
    )
}

/// Fail with 403 `read_only` if the team was not kept when Premium ended.
pub fn ensure_team_editable(conn: &Connection, team_id: i64) -> crate::error::Result<()> {
    if team_locked(conn, team_id)? { Err(read_only_error()) } else { Ok(()) }
}

/// Fail with 403 `read_only` (team not kept) or `limit_choreographies` if
/// `adding` more would pass the team's limit (read-only ones do not count).
pub fn ensure_choreography_room(conn: &Connection, team_id: i64, adding: u32) -> crate::error::Result<()> {
    ensure_team_editable(conn, team_id)?;
    if let Some(limit) = choreography_limit(conn, team_id)?
        && editable_choreography_count(conn, team_id)? + adding > limit
    {
        return Err(crate::error::Error::with_code(
            axum::http::StatusCode::FORBIDDEN,
            "limit_choreographies",
            format!("a team on this plan holds at most {limit} choreographies"),
        ));
    }
    Ok(())
}

/// Bring what an account owns in line with its plan; call after its premium
/// or admin flag changed (and when it is loaded, to settle a pending choice
/// that deleting things made moot).
///
/// - Premium or admin: everything it owns is editable again.
/// - Free, within the free limits (counting what is editable): nothing to do.
/// - Free, over them (Premium just ended): every team it owns becomes
///   read-only and it must pick what to keep ([`keep`]).
/// - Free with a choice pending, but now owning no more than the free plan
///   holds (it deleted the rest): all editable, no choice needed.
pub fn sync_plan(conn: &Connection, account_id: i64) -> rusqlite::Result<()> {
    let row: Option<(bool, bool, bool)> = conn
        .query_row("SELECT is_premium, is_admin, must_choose FROM accounts WHERE id = ?1", [account_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((premium, admin, pending)) = row else { return Ok(()) };
    if premium || admin {
        return unlock_all(conn, account_id);
    }
    let (max_teams, max_choreos) = (arrus_api::FREE_TEAMS, arrus_api::FREE_CHOREOGRAPHIES);
    // (teams, the most choreographies in one of them), all or editable only
    let fits = |editable_only: bool| -> rusqlite::Result<bool> {
        let (t, c) = if editable_only { ("AND NOT t.locked", "AND NOT c.locked") } else { ("", "") };
        let teams: i64 =
            conn.query_row(&format!("SELECT COUNT(*) FROM teams t WHERE t.owner_id = ?1 {t}"), [account_id], |r| r.get(0))?;
        let most: i64 = conn.query_row(
            &format!(
                "SELECT COALESCE(MAX(n), 0) FROM (SELECT COUNT(c.id) AS n FROM teams t \
                 LEFT JOIN choreographies c ON c.team_id = t.id {c} WHERE t.owner_id = ?1 {t} GROUP BY t.id)"
            ),
            [account_id],
            |r| r.get(0),
        )?;
        Ok(teams as u32 <= max_teams && most as u32 <= max_choreos)
    };
    if pending {
        if fits(false)? {
            unlock_all(conn, account_id)?;
        }
    } else if !fits(true)? {
        conn.execute("UPDATE teams SET locked = 1 WHERE owner_id = ?1", [account_id])?;
        conn.execute("UPDATE accounts SET must_choose = 1 WHERE id = ?1", [account_id])?;
    }
    Ok(())
}

fn unlock_all(conn: &Connection, account_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE choreographies SET locked = 0 WHERE locked AND team_id IN (SELECT id FROM teams WHERE owner_id = ?1)",
        [account_id],
    )?;
    conn.execute("UPDATE teams SET locked = 0 WHERE locked AND owner_id = ?1", [account_id])?;
    conn.execute("UPDATE accounts SET must_choose = 0 WHERE must_choose AND id = ?1", [account_id])?;
    Ok(())
}

/// The downgraded account's pick: `team` (one it owns) and exactly
/// `min(free limit, its choreographies)` of them stay editable; every other
/// team it owns, and the team's other choreographies, are read-only.
pub fn keep(conn: &mut Connection, account_id: i64, choice: &arrus_api::KeepChoice) -> crate::error::Result<()> {
    use crate::error::Error;
    use axum::http::StatusCode;
    let bad = |m: &str| Error::with_code(StatusCode::BAD_REQUEST, "bad_choice", m.to_string());
    let pending: bool = conn.query_row("SELECT must_choose FROM accounts WHERE id = ?1", [account_id], |r| r.get(0))?;
    if !pending {
        return Err(Error::with_code(StatusCode::CONFLICT, "no_choice_pending", "there is nothing to choose"));
    }
    let owned: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM teams WHERE owner_id = ?1")?;
        stmt.query_map([account_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let tx = conn.transaction()?;
    match choice.team_id {
        None if owned.is_empty() => {}
        None => return Err(bad("choose the team to keep")),
        Some(team) => {
            if !owned.contains(&team) {
                return Err(bad("you can only keep a team you own"));
            }
            let in_team: Vec<i64> = {
                let mut stmt = tx.prepare("SELECT id FROM choreographies WHERE team_id = ?1")?;
                stmt.query_map([team], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            let mut picked = choice.choreographies.clone();
            picked.sort_unstable();
            picked.dedup();
            let want = in_team.len().min(arrus_api::FREE_CHOREOGRAPHIES as usize);
            if picked.len() != choice.choreographies.len() || picked.len() != want {
                return Err(bad(&format!("choose {want} choreograph{} of this team", if want == 1 { "y" } else { "ies" })));
            }
            if !picked.iter().all(|c| in_team.contains(c)) {
                return Err(bad("those choreographies are not all in this team"));
            }
            tx.execute("UPDATE teams SET locked = (id != ?2) WHERE owner_id = ?1", params![account_id, team])?;
            tx.execute("UPDATE choreographies SET locked = 1 WHERE team_id = ?1", [team])?;
            for c in &picked {
                tx.execute("UPDATE choreographies SET locked = 0 WHERE id = ?1", [c])?;
            }
        }
    }
    tx.execute("UPDATE accounts SET must_choose = 0 WHERE id = ?1", [account_id])?;
    tx.commit()?;
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
