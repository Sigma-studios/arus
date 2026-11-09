//! Sessions, passwords, the login check, and the [`Auth`] extractor with permission helpers.

use arrus_api::{ADMIN_USERNAME, TeamRole};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
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
    /// The master admin (password from the environment).
    Admin,
    Account(Account),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub nickname: String,
    pub is_admin: bool,
    pub is_premium: bool,
    /// The teams it is in, and as what.
    pub teams: Vec<(i64, TeamRole)>,
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
    /// The master admin, or an account with admin rights.
    pub fn is_admin(&self) -> bool {
        match self {
            User::Admin => true,
            User::Account(a) => a.is_admin,
        }
    }

    pub fn username(&self) -> String {
        match self {
            User::Admin => ADMIN_USERNAME.to_string(),
            User::Account(a) => a.nickname.clone(),
        }
    }

    pub fn account_id(&self) -> Option<i64> {
        match self {
            User::Admin => None,
            User::Account(a) => Some(a.id),
        }
    }

    pub fn role_in(&self, team_id: i64) -> Option<TeamRole> {
        match self {
            User::Admin => None,
            User::Account(a) => a.teams.iter().find(|(t, _)| *t == team_id).map(|(_, r)| *r),
        }
    }

    /// The team meant when a request names none: an account's only team.
    pub fn default_team(&self) -> Option<i64> {
        match self {
            User::Account(a) if a.teams.len() == 1 => Some(a.teams[0].0),
            _ => None,
        }
    }

    pub fn require_admin(&self) -> Result<()> {
        if self.is_admin() { Ok(()) } else { Err(Error::forbidden("admin only")) }
    }

    /// The admin, or anyone in `team_id`; others get 404.
    pub fn check_read(&self, team_id: i64) -> Result<()> {
        if self.is_admin() || self.role_in(team_id).is_some() { Ok(()) } else { Err(Error::not_found()) }
    }

    /// The admin, or an owner / co-owner of `team_id` (404 for other teams, 403 for guests).
    pub fn check_write(&self, team_id: i64) -> Result<()> {
        self.check_read(team_id)?;
        match self.role_in(team_id) {
            Some(TeamRole::Guest) if !self.is_admin() => {
                Err(Error::forbidden("you can view this team's choreographies but not change them"))
            }
            _ => Ok(()),
        }
    }

    /// The admin, or the owner of `team_id`: may rename it, delete it and
    /// decide who is in it.
    pub fn check_manage(&self, team_id: i64) -> Result<()> {
        self.check_read(team_id)?;
        if self.is_admin() || self.role_in(team_id) == Some(TeamRole::Owner) {
            Ok(())
        } else {
            Err(Error::forbidden("only the team's owner can do that"))
        }
    }
}

/// An argon2 hash (PHC string) of `password`.
pub fn hash_password(password: &str) -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    let Ok(salt) = SaltString::encode_b64(&bytes) else { return String::new() };
    Argon2::default().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// Check an account's password. A password still visible to the admin
/// (generated, or set by the admin) is compared ignoring case: phone
/// keyboards like to capitalise.
pub fn password_matches(typed: &str, hash: &str, visible: Option<&str>) -> bool {
    match visible {
        Some(v) => ct_eq(&typed.trim().to_lowercase(), &v.to_lowercase()),
        None => verify_password(typed, hash),
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
pub fn create_session(conn: &Connection, account_id: Option<i64>) -> rusqlite::Result<String> {
    let token = new_token();
    let now = db::now();
    conn.execute("DELETE FROM sessions WHERE expires_at < ?1", [now])?;
    conn.execute(
        "INSERT INTO sessions (token_hash, account_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![hash_token(&token), account_id, now, now + SESSION_TTL],
    )?;
    Ok(token)
}

/// `Some(account_id)` (None inside = the master admin) if the credentials are
/// right. `login` is a nickname or an email.
pub fn check_login(
    conn: &Connection,
    admin_password: &str,
    login: &str,
    password: &str,
) -> rusqlite::Result<Option<Option<i64>>> {
    let login = login.trim();
    if login.eq_ignore_ascii_case(ADMIN_USERNAME) {
        return Ok(ct_eq(password, admin_password).then_some(None));
    }
    let lower = login.to_lowercase();
    let row: Option<(i64, String, Option<String>)> = conn
        .query_row(
            "SELECT id, password_hash, password_visible FROM accounts WHERE nickname_lower = ?1 OR email_lower = ?1",
            [&lower],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        Some((id, hash, visible)) if password_matches(password, &hash, visible.as_deref()) => Some(Some(id)),
        Some(_) => None,
        None => {
            // About as slow as a real check.
            let _ = verify_password(password, &DUMMY_HASH);
            None
        }
    })
}

static DUMMY_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| hash_password("no such account"));

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
    let row: Option<(Option<i64>, i64)> = conn
        .query_row("SELECT account_id, expires_at FROM sessions WHERE token_hash = ?1", [&token_hash], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((account_id, expires_at)) = row else {
        return Ok(None);
    };
    let now = db::now();
    if expires_at < now {
        conn.execute("DELETE FROM sessions WHERE token_hash = ?1", [&token_hash])?;
        return Ok(None);
    }
    let user = match account_id {
        None => User::Admin,
        Some(id) => match load_account(conn, id)? {
            Some(a) => User::Account(a),
            None => return Ok(None),
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

pub fn load_account(conn: &Connection, id: i64) -> rusqlite::Result<Option<Account>> {
    let row: Option<(String, bool, bool)> = conn
        .query_row("SELECT nickname, is_admin, is_premium FROM accounts WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((nickname, is_admin, is_premium)) = row else { return Ok(None) };
    let mut stmt = conn.prepare("SELECT team_id, role FROM team_members WHERE account_id = ?1 ORDER BY team_id")?;
    let teams = stmt
        .query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| match r {
            Ok((t, role)) => TeamRole::parse(&role).map(|role| Ok((t, role))),
            Err(e) => Some(Err(e)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(Account { id, nickname, is_admin, is_premium, teams }))
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
