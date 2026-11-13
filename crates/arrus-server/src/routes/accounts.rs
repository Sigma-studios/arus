//! Accounts: who is logged in (and changing their own details), and the
//! admin's list of every account.

use arrus_api::{
    ADMIN_USERNAME, AccountInfo, AccountTeam, ChoreographyRef, CreateAccount, DeleteMe, KeepChoice, MIN_PASSWORD_LEN, Me, MyTeam,
    PlanLimits,
    NICKNAME_CHANGE_INTERVAL, Role, TeamRole, UpdateAccount, UpdateMe, UserPrefs,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};

use super::ApiJson;
use super::teams::delete_teams;
use crate::auth::{self, Auth, User};
use crate::error::{Error, Result};
use crate::{AppState, db, email, words};

// ---- validation -------------------------------------------------------------

/// Trimmed, 1–40 chars of letters/digits/`._-`, not `admin`.
pub fn clean_nickname(nickname: &str) -> Result<String> {
    let u = nickname.trim();
    if u.is_empty() || u.chars().count() > 40 {
        return Err(Error::bad_request("the nickname must be 1 to 40 characters"));
    }
    if !u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(Error::bad_request("the nickname may only contain letters, digits, '.', '_' and '-'"));
    }
    if u.eq_ignore_ascii_case(ADMIN_USERNAME) {
        return Err(Error::with_code(StatusCode::CONFLICT, "username_taken", "that nickname is reserved"));
    }
    Ok(u.to_string())
}

/// A trimmed address that looks like `name@host.tld`; `None` for empty.
pub fn clean_email(email: &str) -> Result<Option<String>> {
    let e = email.trim();
    if e.is_empty() {
        return Ok(None);
    }
    let ok = e.len() <= 254
        && !e.chars().any(|c| c.is_whitespace() || c.is_control())
        && e.split_once('@').is_some_and(|(user, host)| {
            !user.is_empty() && !host.contains('@') && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
        });
    if !ok {
        return Err(Error::bad_request("that does not look like an email address"));
    }
    Ok(Some(e.to_string()))
}

pub fn check_new_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(Error::bad_request(format!("the password must be at least {MIN_PASSWORD_LEN} characters")));
    }
    if password.chars().count() > 200 {
        return Err(Error::bad_request("the password is too long"));
    }
    Ok(())
}

pub fn nickname_free(conn: &Connection, nickname: &str, except: Option<i64>) -> Result<()> {
    let taken: Option<i64> = conn
        .query_row(
            "SELECT id FROM accounts WHERE nickname_lower = ?1 AND id IS NOT ?2",
            params![nickname.to_lowercase(), except],
            |r| r.get(0),
        )
        .optional()?;
    if taken.is_some() {
        return Err(Error::with_code(StatusCode::CONFLICT, "username_taken", "that nickname is already taken"));
    }
    Ok(())
}

pub fn email_free(conn: &Connection, email: &str, except: Option<i64>) -> Result<()> {
    let taken: Option<i64> = conn
        .query_row(
            "SELECT id FROM accounts WHERE email_lower = ?1 AND id IS NOT ?2",
            params![email.to_lowercase(), except],
            |r| r.get(0),
        )
        .optional()?;
    if taken.is_some() {
        return Err(Error::with_code(StatusCode::CONFLICT, "email_taken", "an account already uses that email"));
    }
    Ok(())
}

/// Insert an account (fields already cleaned); returns its id. `visible`:
/// keep the password readable for the admin (generated or admin-set).
pub fn insert_account(
    conn: &Connection,
    nickname: &str,
    email: Option<&str>,
    password: &str,
    visible: bool,
) -> Result<i64> {
    nickname_free(conn, nickname, None)?;
    if let Some(e) = email {
        email_free(conn, e, None)?;
    }
    conn.execute(
        "INSERT INTO accounts (nickname, nickname_lower, email, email_lower, password_hash, password_visible, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            nickname,
            nickname.to_lowercase(),
            email,
            email.map(str::to_lowercase),
            auth::hash_password(password),
            visible.then_some(password),
            db::now()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Set an account's email; a different address is no longer confirmed.
/// Returns whether it changed.
fn set_email(conn: &Connection, id: i64, email: Option<&str>) -> Result<bool> {
    let lower = email.map(str::to_lowercase);
    let current: Option<String> = conn.query_row("SELECT email_lower FROM accounts WHERE id = ?1", [id], |r| r.get(0))?;
    let changed = current != lower;
    conn.execute("UPDATE accounts SET email = ?1, email_lower = ?2 WHERE id = ?3", params![email, lower, id])?;
    if changed {
        conn.execute("UPDATE accounts SET email_verified_at = NULL WHERE id = ?1", [id])?;
    }
    Ok(changed && email.is_some())
}

/// After an account changed: tell its old address that the email changed,
/// and (`password`) that the password changed.
fn notify_changes(
    state: &AppState,
    conn: &Connection,
    id: i64,
    before: Option<(String, Option<String>)>,
    password: bool,
) -> Result<()> {
    let Some((_, old)) = before else { return Ok(()) };
    let Some((nickname, now)) = email::contact_of(conn, id)? else { return Ok(()) };
    if let Some(old) = &old
        && Some(old.to_lowercase()) != now.as_deref().map(str::to_lowercase)
    {
        email::notify(state, old, &nickname, email::Notice::EmailChanged { new_email: now.clone() });
    }
    // To the address it had before: a thief changing both is still reported.
    if password && let Some(to) = old.or(now) {
        email::notify(state, &to, &nickname, email::Notice::PasswordChanged);
    }
    Ok(())
}

// ---- me ---------------------------------------------------------------------

fn my_teams(conn: &Connection, account_id: i64) -> Result<Vec<MyTeam>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, m.role, a.nickname, \
         (SELECT COUNT(*) FROM choreographies c WHERE c.team_id = t.id), \
         t.owner_id, a.is_premium, a.is_admin, \
         (SELECT COALESCE(SUM(c.music_bytes), 0) FROM choreographies c JOIN teams o ON o.id = c.team_id \
          WHERE o.owner_id = t.owner_id), \
         t.locked \
         FROM team_members m JOIN teams t ON t.id = m.team_id LEFT JOIN accounts a ON a.id = t.owner_id \
         WHERE m.account_id = ?1 ORDER BY t.name COLLATE NOCASE, t.id",
    )?;
    let teams = stmt
        .query_map([account_id], |r| {
            let role: String = r.get(2)?;
            let owner: Option<i64> = r.get(5)?;
            let plan = match (owner, r.get::<_, Option<bool>>(6)?, r.get::<_, Option<bool>>(7)?) {
                (Some(_), Some(p), Some(a)) => PlanLimits::of(p, a),
                _ => PlanLimits::UNLIMITED,
            };
            Ok(MyTeam {
                id: r.get(0)?,
                name: r.get(1)?,
                role: TeamRole::parse(&role).unwrap_or(TeamRole::Guest),
                owner: r.get(3)?,
                choreography_count: r.get::<_, i64>(4)? as u32,
                choreography_limit: plan.choreographies_per_team,
                music_limit: plan.music_bytes,
                music_used: if owner.is_some() { r.get::<_, i64>(8)? as u64 } else { 0 },
                read_only: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(teams)
}

fn load_me(state: &AppState, conn: &Connection, user: &User) -> Result<Me> {
    let mail = state.mailer.is_some();
    let User::Account(a) = user else {
        return Ok(Me {
            username: ADMIN_USERNAME.into(),
            role: Role::Admin,
            account_id: None,
            email: None,
            email_verified: false,
            mail,
            confirm_sent: false,
            premium: true,
            teams: Vec::new(),
            can_create_team: true,
            limits: PlanLimits::UNLIMITED,
            teams_owned: 0,
            music_used: 0,
            nickname_change_from: None,
            prefs: admin_prefs(conn)?,
            must_choose: false,
        });
    };
    // A pending choice may have become moot (the rest was deleted).
    db::sync_plan(conn, a.id)?;
    let (email, changed_at, prefs, verified_at, must_choose): (Option<String>, Option<i64>, Option<String>, Option<i64>, bool) =
        conn.query_row(
            "SELECT email, nickname_changed_at, prefs, email_verified_at, must_choose FROM accounts WHERE id = ?1",
            [a.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    let owned: i64 = conn.query_row("SELECT COUNT(*) FROM teams WHERE owner_id = ?1", [a.id], |r| r.get(0))?;
    // read-only teams do not count toward the limit
    let owned_editable: i64 =
        conn.query_row("SELECT COUNT(*) FROM teams WHERE owner_id = ?1 AND NOT locked", [a.id], |r| r.get(0))?;
    let limits = PlanLimits::of(a.is_premium, a.is_admin);
    Ok(Me {
        username: a.nickname.clone(),
        role: if a.is_admin { Role::Admin } else { Role::Member },
        account_id: Some(a.id),
        email_verified: email.is_some() && verified_at.is_some(),
        confirm_sent: email::pending(conn, a.id, email::Kind::Confirm)?,
        email,
        mail,
        premium: a.is_premium,
        teams: my_teams(conn, a.id)?,
        can_create_team: !must_choose && limits.teams.is_none_or(|l| (owned_editable as u32) < l),
        limits,
        teams_owned: owned as u32,
        music_used: db::music_used(conn, a.id)?,
        nickname_change_from: nickname_change_from(changed_at, db::now()),
        prefs: parse_prefs(prefs.as_deref()),
        must_choose,
    })
}

/// When the nickname may change again, if not yet.
fn nickname_change_from(changed_at: Option<i64>, now: i64) -> Option<i64> {
    changed_at.map(|t| t + NICKNAME_CHANGE_INTERVAL).filter(|&from| from > now)
}

fn parse_prefs(json: Option<&str>) -> UserPrefs {
    json.and_then(|j| serde_json::from_str::<UserPrefs>(j).ok()).unwrap_or_default().cleaned()
}

/// The master admin has no account row: their settings live in `meta`.
const ADMIN_PREFS_KEY: &str = "admin_prefs";

fn admin_prefs(conn: &Connection) -> Result<UserPrefs> {
    let json: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [ADMIN_PREFS_KEY], |r| r.get(0)).optional()?;
    Ok(parse_prefs(json.as_deref()))
}

/// Save the logged-in person's settings (no password needed).
pub async fn update_prefs(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(body): ApiJson<UserPrefs>,
) -> Result<Json<UserPrefs>> {
    let prefs = body.cleaned();
    let json = serde_json::to_string(&prefs).map_err(|e| Error::bad_request(e.to_string()))?;
    let id = auth.user.account_id();
    state
        .db(move |_, conn| {
            match id {
                Some(id) => conn.execute("UPDATE accounts SET prefs = ?1 WHERE id = ?2", params![json, id])?,
                None => conn.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![ADMIN_PREFS_KEY, json],
                )?,
            };
            Ok(Json(prefs))
        })
        .await
}

pub async fn me(State(state): State<AppState>, auth: Auth) -> Result<Json<Me>> {
    state.db(move |s, conn| Ok(Json(load_me(s, conn, &auth.user)?))).await
}

/// After Premium ended: the team and choreographies that stay editable.
pub async fn keep(State(state): State<AppState>, auth: Auth, ApiJson(body): ApiJson<KeepChoice>) -> Result<Json<Me>> {
    let Some(id) = auth.user.account_id() else {
        return Err(Error::with_code(StatusCode::CONFLICT, "no_choice_pending", "there is nothing to choose"));
    };
    state
        .db(move |s, conn| {
            db::keep(conn, id, &body)?;
            let user = auth::load_account(conn, id)?.map(User::Account).ok_or_else(Error::not_found)?;
            Ok(Json(load_me(s, conn, &user)?))
        })
        .await
}

/// Send the confirm link (again) to the logged-in account's email.
pub async fn confirm_email(State(state): State<AppState>, auth: Auth) -> Result<StatusCode> {
    let Some(id) = auth.user.account_id() else {
        return Err(Error::with_code(StatusCode::BAD_REQUEST, "no_email", "the admin has no email"));
    };
    let sent = state.db(move |s, conn| email::send(s, conn, id, email::Kind::Confirm)).await?;
    match sent {
        email::Sent::Sent => Ok(StatusCode::NO_CONTENT),
        email::Sent::TooSoon => Err(Error::with_code(
            StatusCode::TOO_MANY_REQUESTS,
            "too_soon",
            "an email was just sent: wait a minute before asking again",
        )),
        email::Sent::NoEmail => Err(Error::with_code(StatusCode::BAD_REQUEST, "no_email", "your account has no email")),
        email::Sent::Off => {
            Err(Error::with_code(StatusCode::SERVICE_UNAVAILABLE, "mail_off", "this server does not send emails"))
        }
    }
}

pub async fn update_me(State(state): State<AppState>, auth: Auth, ApiJson(body): ApiJson<UpdateMe>) -> Result<Json<Me>> {
    let Some(id) = auth.user.account_id() else {
        return Err(Error::bad_request("the admin's password is set on the server"));
    };
    let nickname = body.nickname.as_deref().map(clean_nickname).transpose()?;
    let email = body.email.as_deref().map(clean_email).transpose()?;
    if let Some(p) = &body.new_password {
        check_new_password(p)?;
    }
    let token_hash = auth.token_hash.clone();
    state
        .db(move |s, conn| {
            let (hash, visible): (String, Option<String>) = conn.query_row(
                "SELECT password_hash, password_visible FROM accounts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if !auth::password_matches(&body.current_password, &hash, visible.as_deref()) {
                return Err(Error::with_code(StatusCode::FORBIDDEN, "wrong_password", "the current password is wrong"));
            }
            let tx = conn.transaction()?;
            if let Some(n) = &nickname {
                let (current, changed_at): (String, Option<i64>) = tx.query_row(
                    "SELECT nickname, nickname_changed_at FROM accounts WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                if *n != current {
                    let now = db::now();
                    if let Some(from) = nickname_change_from(changed_at, now) {
                        let days = (from - now + 86_399) / 86_400;
                        return Err(Error::with_code(
                            StatusCode::TOO_MANY_REQUESTS,
                            "nickname_too_soon",
                            format!(
                                "the nickname can be changed once a month: again in {days} day{}",
                                if days == 1 { "" } else { "s" }
                            ),
                        ));
                    }
                    nickname_free(&tx, n, Some(id))?;
                    tx.execute(
                        "UPDATE accounts SET nickname = ?1, nickname_lower = ?2, nickname_changed_at = ?3 WHERE id = ?4",
                        params![n, n.to_lowercase(), now, id],
                    )?;
                }
            }
            let before = email::contact_of(&tx, id)?;
            let mut email_changed = false;
            if let Some(e) = &email {
                if let Some(e) = e {
                    email_free(&tx, e, Some(id))?;
                }
                email_changed = set_email(&tx, id, e.as_deref())?;
            }
            if let Some(p) = &body.new_password {
                // Their own password: hashed only, the admin no longer sees it.
                tx.execute(
                    "UPDATE accounts SET password_hash = ?1, password_visible = NULL WHERE id = ?2",
                    params![auth::hash_password(p), id],
                )?;
                tx.execute("DELETE FROM sessions WHERE account_id = ?1 AND token_hash != ?2", params![id, token_hash])?;
            }
            tx.commit()?;
            if email_changed {
                email::send(s, conn, id, email::Kind::Confirm)?;
            }
            notify_changes(s, conn, id, before, body.new_password.is_some())?;
            let user = User::Account(auth::load_account(conn, id)?.ok_or_else(Error::not_found)?);
            Ok(Json(load_me(s, conn, &user)?))
        })
        .await
}

// ---- admin ------------------------------------------------------------------

const ACCOUNT_SELECT: &str =
    "SELECT id, nickname, email, password_visible, is_admin, is_premium, created_at FROM accounts";

fn account_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AccountInfo> {
    Ok(AccountInfo {
        id: r.get(0)?,
        nickname: r.get(1)?,
        email: r.get(2)?,
        password: r.get(3)?,
        is_admin: r.get(4)?,
        is_premium: r.get(5)?,
        created_at: r.get(6)?,
        teams: Vec::new(),
    })
}

/// Every account with its teams and their choreographies.
fn load_accounts(conn: &Connection, only: Option<i64>) -> Result<Vec<AccountInfo>> {
    let mut stmt = conn.prepare(&format!("{ACCOUNT_SELECT} WHERE ?1 IS NULL OR id = ?1 ORDER BY nickname_lower"))?;
    let mut accounts = stmt.query_map([only], account_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare(
        "SELECT m.account_id, t.id, t.name, m.role FROM team_members m JOIN teams t ON t.id = m.team_id \
         WHERE ?1 IS NULL OR m.account_id = ?1 \
         ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'co_owner' THEN 1 ELSE 2 END, t.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([only], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut choreos = conn.prepare("SELECT id, name FROM choreographies WHERE team_id = ?1 ORDER BY name COLLATE NOCASE")?;
    for (account, team_id, name, role) in rows {
        let Some(a) = accounts.iter_mut().find(|a| a.id == account) else { continue };
        let choreographies = choreos
            .query_map([team_id], |r| Ok(ChoreographyRef { id: r.get(0)?, name: r.get(1)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        a.teams.push(AccountTeam {
            team_id,
            name,
            role: TeamRole::parse(&role).unwrap_or(TeamRole::Guest),
            choreographies,
        });
    }
    Ok(accounts)
}

fn load_account_info(conn: &Connection, id: i64) -> Result<AccountInfo> {
    load_accounts(conn, Some(id))?.into_iter().next().ok_or_else(Error::not_found)
}

pub async fn list(State(state): State<AppState>, auth: Auth) -> Result<Json<Vec<AccountInfo>>> {
    auth.user.require_admin()?;
    state.db(|_, conn| Ok(Json(load_accounts(conn, None)?))).await
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(body): ApiJson<CreateAccount>,
) -> Result<Json<AccountInfo>> {
    auth.user.require_admin()?;
    let nickname = clean_nickname(&body.nickname)?;
    let email = body.email.as_deref().map(clean_email).transpose()?.flatten();
    let password = match body.password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => {
            check_new_password(p)?;
            p.to_string()
        }
        None => words::generate_password(),
    };
    state
        .db(move |_, conn| {
            let id = insert_account(conn, &nickname, email.as_deref(), &password, true)?;
            Ok(Json(load_account_info(conn, id)?))
        })
        .await
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<UpdateAccount>,
) -> Result<Json<AccountInfo>> {
    auth.user.require_admin()?;
    let nickname = body.nickname.as_deref().map(clean_nickname).transpose()?;
    let email = body.email.as_deref().map(clean_email).transpose()?;
    let password = match (body.password.as_deref().map(str::trim), body.regenerate_password) {
        (Some(p), _) if !p.is_empty() => {
            check_new_password(p)?;
            Some(p.to_string())
        }
        (_, true) => Some(words::generate_password()),
        _ => None,
    };
    if body.is_admin == Some(false) && auth.user.account_id() == Some(id) {
        return Err(Error::bad_request("you cannot take away your own admin rights"));
    }
    state
        .db(move |s, conn| {
            let tx = conn.transaction()?;
            let exists: Option<i64> = tx.query_row("SELECT id FROM accounts WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            if exists.is_none() {
                return Err(Error::not_found());
            }
            if let Some(n) = &nickname {
                nickname_free(&tx, n, Some(id))?;
                tx.execute("UPDATE accounts SET nickname = ?1, nickname_lower = ?2 WHERE id = ?3", params![n, n.to_lowercase(), id])?;
            }
            let before = email::contact_of(&tx, id)?;
            let was_premium: bool = tx.query_row("SELECT is_premium FROM accounts WHERE id = ?1", [id], |r| r.get(0))?;
            let premium_ended = was_premium && body.is_premium == Some(false);
            let mut email_changed = false;
            if let Some(e) = &email {
                if let Some(e) = e {
                    email_free(&tx, e, Some(id))?;
                }
                email_changed = set_email(&tx, id, e.as_deref())?;
            }
            if let Some(v) = body.is_admin {
                tx.execute("UPDATE accounts SET is_admin = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = body.is_premium {
                tx.execute("UPDATE accounts SET is_premium = ?1 WHERE id = ?2", params![v, id])?;
            }
            if body.is_premium.is_some() || body.is_admin.is_some() {
                db::sync_plan(&tx, id)?;
            }
            if let Some(p) = &password {
                tx.execute(
                    "UPDATE accounts SET password_hash = ?1, password_visible = ?2 WHERE id = ?3",
                    params![auth::hash_password(p), p, id],
                )?;
                tx.execute("DELETE FROM sessions WHERE account_id = ?1", [id])?;
            }
            tx.commit()?;
            if email_changed {
                email::send(s, conn, id, email::Kind::Confirm)?;
            }
            // A password the admin sets is handed over by the admin: no email.
            notify_changes(s, conn, id, before, false)?;
            if premium_ended {
                email::premium_ended(s, conn, id)?;
            }
            Ok(Json(load_account_info(conn, id)?))
        })
        .await
}

/// Delete an account: the teams it owns (with their choreographies and
/// music), then the account (sessions, memberships, email links and activity
/// cascade); a last email confirms it.
pub fn delete_account(s: &AppState, conn: &mut Connection, id: i64) -> Result<()> {
    let Some((nickname, address)) = email::contact_of(conn, id)? else { return Err(Error::not_found()) };
    let owned: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM teams WHERE owner_id = ?1")?;
        stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    delete_teams(s, conn, &owned)?;
    if conn.execute("DELETE FROM accounts WHERE id = ?1", [id])? == 0 {
        return Err(Error::not_found());
    }
    if let Some(to) = address {
        email::notify(s, &to, &nickname, email::Notice::AccountDeleted);
    }
    Ok(())
}

/// The logged-in account deletes itself (password needed).
pub async fn delete_me(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(body): ApiJson<DeleteMe>,
) -> Result<axum::response::Response> {
    let Some(id) = auth.user.account_id() else {
        return Err(Error::bad_request("the master admin has no account to delete"));
    };
    state
        .db(move |s, conn| {
            let (hash, visible): (String, Option<String>) = conn.query_row(
                "SELECT password_hash, password_visible FROM accounts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if !auth::password_matches(&body.password, &hash, visible.as_deref()) {
                return Err(Error::with_code(StatusCode::FORBIDDEN, "wrong_password", "the password is wrong"));
            }
            delete_account(s, conn, id)
        })
        .await?;
    let mut r = axum::response::IntoResponse::into_response(StatusCode::NO_CONTENT);
    auth::set_cookie(r.headers_mut(), auth::clear_cookie(&state));
    Ok(r)
}

/// Deletes the account, its sessions and the teams it owns.
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<StatusCode> {
    auth.user.require_admin()?;
    if auth.user.account_id() == Some(id) {
        return Err(Error::bad_request("you cannot delete your own account"));
    }
    state
        .db(move |s, conn| {
            delete_account(s, conn, id)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}
