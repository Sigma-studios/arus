//! Sessions, the login check, and the [`Auth`] extractor with permission helpers.

use arrus_api::{ADMIN_USERNAME, Me, Role, TeamRef};
use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use rand::RngExt;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};
use crate::{AppState, db};

pub const COOKIE_NAME: &str = "arus_session";
/// Session lifetime (sliding: extended when used after a day).
pub const SESSION_TTL: i64 = 30 * 24 * 3600;
const SLIDE_AFTER: i64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum User {
    Admin,
    Member { id: i64, username: String, team_id: i64, team_name: String, can_write: bool },
}

/// The logged-in user of a request.
#[derive(Debug, Clone)]
pub struct Auth {
    pub user: User,
    pub token_hash: String,
    /// The session's expiry was just extended; pages may re-send the cookie.
    pub refreshed: bool,
    /// The raw token (to re-send the cookie).
    pub token: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        matches!(self, User::Admin)
    }

    pub fn username(&self) -> String {
        match self {
            User::Admin => ADMIN_USERNAME.to_string(),
            User::Member { username, .. } => username.clone(),
        }
    }

    pub fn me(&self) -> Me {
        match self {
            User::Admin => Me { username: ADMIN_USERNAME.into(), role: Role::Admin, team: None, can_write: true },
            User::Member { username, team_id, team_name, can_write, .. } => Me {
                username: username.clone(),
                role: Role::Member,
                team: Some(TeamRef { id: *team_id, name: team_name.clone() }),
                can_write: *can_write,
            },
        }
    }

    pub fn require_admin(&self) -> Result<()> {
        if self.is_admin() { Ok(()) } else { Err(Error::forbidden("admin only")) }
    }

    /// The admin, or a member of `team_id`; others get 404.
    pub fn check_read(&self, team_id: i64) -> Result<()> {
        match self {
            User::Admin => Ok(()),
            User::Member { team_id: t, .. } if *t == team_id => Ok(()),
            User::Member { .. } => Err(Error::not_found()),
        }
    }

    /// The admin, or a writer of `team_id` (404 for other teams, 403 for readers).
    pub fn check_write(&self, team_id: i64) -> Result<()> {
        self.check_read(team_id)?;
        match self {
            User::Member { can_write: false, .. } => {
                Err(Error::forbidden("you can view this team's choreographies but not change them"))
            }
            _ => Ok(()),
        }
    }
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Constant-time string comparison (hashing first hides the length too).
pub fn ct_eq(a: &str, b: &str) -> bool {
    let ha = Sha256::digest(a.as_bytes());
    let hb = Sha256::digest(b.as_bytes());
    ha.as_slice().ct_eq(hb.as_slice()).into()
}

/// Create a session; returns the raw token for the cookie.
pub fn create_session(conn: &Connection, member_id: Option<i64>) -> rusqlite::Result<String> {
    let token = new_token();
    let now = db::now();
    conn.execute("DELETE FROM sessions WHERE expires_at < ?1", [now])?;
    conn.execute(
        "INSERT INTO sessions (token_hash, member_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![hash_token(&token), member_id, now, now + SESSION_TTL],
    )?;
    Ok(token)
}

/// `Some(member_id)` (None inside = admin) if the credentials are right.
pub fn check_login(
    conn: &Connection,
    admin_password: &str,
    username: &str,
    password: &str,
) -> rusqlite::Result<Option<Option<i64>>> {
    let username = username.trim();
    if username.eq_ignore_ascii_case(ADMIN_USERNAME) {
        return Ok(ct_eq(password, admin_password).then_some(None));
    }
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, password FROM members WHERE username_lower = ?1",
            [username.to_lowercase()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    // Generated passwords are lowercase; phone keyboards like to capitalise.
    let typed = password.trim().to_lowercase();
    Ok(match row {
        Some((id, stored)) if ct_eq(&typed, &stored) => Some(Some(id)),
        Some(_) => None,
        None => {
            let _ = ct_eq(&typed, "dummy-dummy-00");
            None
        }
    })
}

pub fn session_token(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(COOKIE) {
        let Ok(s) = value.to_str() else { continue };
        for pair in s.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=')
                && k.trim() == COOKIE_NAME
            {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Look up (and slide) the session of a token.
pub fn lookup(conn: &Connection, token: &str) -> rusqlite::Result<Option<Auth>> {
    let token_hash = hash_token(token);
    type Row = (Option<i64>, i64, Option<String>, Option<i64>, Option<String>, Option<bool>);
    let row: Option<Row> = conn
        .query_row(
            "SELECT s.member_id, s.expires_at, m.username, m.team_id, t.name, m.can_write \
             FROM sessions s LEFT JOIN members m ON m.id = s.member_id \
             LEFT JOIN teams t ON t.id = m.team_id WHERE s.token_hash = ?1",
            [&token_hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((member_id, expires_at, username, team_id, team_name, can_write)) = row else {
        return Ok(None);
    };
    let now = db::now();
    if expires_at < now {
        conn.execute("DELETE FROM sessions WHERE token_hash = ?1", [&token_hash])?;
        return Ok(None);
    }
    let user = match member_id {
        None => User::Admin,
        Some(id) => match (username, team_id, team_name, can_write) {
            (Some(username), Some(team_id), Some(team_name), Some(can_write)) => {
                User::Member { id, username, team_id, team_name, can_write }
            }
            _ => return Ok(None),
        },
    };
    let refreshed = expires_at - now < SESSION_TTL - SLIDE_AFTER;
    if refreshed {
        conn.execute(
            "UPDATE sessions SET expires_at = ?1 WHERE token_hash = ?2",
            params![now + SESSION_TTL, &token_hash],
        )?;
    }
    Ok(Some(Auth { user, token_hash, refreshed, token: token.to_string() }))
}

/// The session of a request, if any.
pub async fn from_headers(state: &AppState, headers: &HeaderMap) -> Result<Option<Auth>> {
    let Some(token) = session_token(headers) else { return Ok(None) };
    state.db(move |_, conn| Ok(lookup(conn, &token)?)).await
}

pub fn session_cookie(state: &AppState, token: &str) -> HeaderValue {
    let secure = if state.config.insecure_cookies { "" } else { "; Secure" };
    let v = format!("{COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_TTL}{secure}");
    HeaderValue::from_str(&v).expect("cookie is ASCII")
}

pub fn clear_cookie(state: &AppState) -> HeaderValue {
    let secure = if state.config.insecure_cookies { "" } else { "; Secure" };
    let v = format!("{COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure}");
    HeaderValue::from_str(&v).expect("cookie is ASCII")
}

pub fn set_cookie(headers: &mut HeaderMap, value: HeaderValue) {
    headers.append(SET_COOKIE, value);
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self> {
        if let Some(auth) = parts.extensions.get::<Auth>() {
            return Ok(auth.clone());
        }
        let auth = from_headers(state, &parts.headers).await?.ok_or_else(Error::unauthorized)?;
        parts.extensions.insert(auth.clone());
        Ok(auth)
    }
}
