//! Links sent by email (confirming an address, resetting a password) and
//! notices (email or password changed, account deleted, Premium ended).
//!
//! Each link carries a random token; only its hash is stored (like sessions).
//! A token works once, until it expires, and only for the address it was
//! sent to (changing the email makes older confirm links useless).

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::{AppState, auth, db, mail};

/// A confirm link works this long.
pub const CONFIRM_TTL: i64 = 24 * 3600;
/// A reset link works this long.
pub const RESET_TTL: i64 = 3600;
/// At most one email of a kind per account per this many seconds.
pub const RESEND_GAP: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Confirm,
    Reset,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Confirm => "confirm",
            Kind::Reset => "reset",
        }
    }

    fn ttl(self) -> i64 {
        match self {
            Kind::Confirm => CONFIRM_TTL,
            Kind::Reset => RESET_TTL,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Kind::Confirm => "/confirm-email/",
            Kind::Reset => "/reset-password/",
        }
    }
}

/// What became of a request to send a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Sent,
    /// One was sent less than [`RESEND_GAP`] ago.
    TooSoon,
    /// The account has no email address.
    NoEmail,
    /// The server sends no emails.
    Off,
}

/// Make a token and email the link to the account's current address.
pub fn send(state: &AppState, conn: &Connection, account_id: i64, kind: Kind) -> Result<Sent> {
    let Some(mailer) = &state.mailer else { return Ok(Sent::Off) };
    let now = db::now();
    conn.execute("DELETE FROM email_tokens WHERE expires_at < ?1", [now])?;
    let row: Option<(String, Option<String>)> = conn
        .query_row("SELECT nickname, email FROM accounts WHERE id = ?1", [account_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((nickname, Some(email))) = row else { return Ok(Sent::NoEmail) };
    let recent: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM email_tokens WHERE account_id = ?1 AND kind = ?2 AND created_at > ?3",
            params![account_id, kind.as_str(), now - RESEND_GAP],
            |r| r.get(0),
        )
        .optional()?;
    if recent.is_some() {
        return Ok(Sent::TooSoon);
    }
    let token = auth::new_token();
    conn.execute(
        "INSERT INTO email_tokens (token_hash, account_id, kind, email_lower, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![auth::hash_token(&token), account_id, kind.as_str(), email.to_lowercase(), now, now + kind.ttl()],
    )?;
    let base = state.config.public_url.as_deref().unwrap_or("");
    let link = format!("{base}{}{token}", kind.path());
    mailer.send(match kind {
        Kind::Confirm => mail::confirm_email(&email, &nickname, &link),
        Kind::Reset => mail::reset_password(&email, &nickname, &link),
    });
    Ok(Sent::Sent)
}

/// The account a token (still) works for, with its nickname.
pub fn find(conn: &Connection, token: &str, kind: Kind) -> Result<Option<(i64, String)>> {
    let row: Option<(i64, String, i64)> = conn
        .query_row(
            "SELECT t.account_id, a.nickname, t.expires_at FROM email_tokens t JOIN accounts a ON a.id = t.account_id \
             WHERE t.token_hash = ?1 AND t.kind = ?2 AND a.email_lower = t.email_lower",
            params![auth::hash_token(token), kind.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(row.filter(|(_, _, exp)| *exp >= db::now()).map(|(id, nick, _)| (id, nick)))
}

/// Whether a link of `kind` sent to the account's current address still works.
pub fn pending(conn: &Connection, account_id: i64, kind: Kind) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM email_tokens t JOIN accounts a ON a.id = t.account_id \
         WHERE t.account_id = ?1 AND t.kind = ?2 AND t.expires_at >= ?3 AND a.email_lower = t.email_lower",
        params![account_id, kind.as_str(), db::now()],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Use up every link of `kind` sent to the account.
pub fn spend(conn: &Connection, account_id: i64, kind: Kind) -> Result<()> {
    conn.execute("DELETE FROM email_tokens WHERE account_id = ?1 AND kind = ?2", params![account_id, kind.as_str()])?;
    Ok(())
}

/// A confirm link was opened: the address is confirmed. `None`: the link is
/// unknown, used, expired, or for an address the account no longer has.
pub fn confirm(conn: &Connection, token: &str) -> Result<Option<String>> {
    let Some((id, nickname)) = find(conn, token, Kind::Confirm)? else { return Ok(None) };
    conn.execute("UPDATE accounts SET email_verified_at = ?1 WHERE id = ?2", params![db::now(), id])?;
    spend(conn, id, Kind::Confirm)?;
    Ok(Some(nickname))
}

/// The account using `email`, if any.
pub fn account_by_email(conn: &Connection, email: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM accounts WHERE email_lower = ?1", [email.trim().to_lowercase()], |r| r.get(0))
        .optional()?)
}

/// An email that only informs (no link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Sent to the old address; the new one (`None`: removed).
    EmailChanged { new_email: Option<String> },
    PasswordChanged,
    AccountDeleted,
    /// Premium ended; `must_choose`: its teams are read-only until it picks what to keep.
    PremiumEnded { must_choose: bool },
}

/// Send a notice to `to` (when emails are on).
pub fn notify(state: &AppState, to: &str, nickname: &str, notice: Notice) {
    let Some(mailer) = &state.mailer else { return };
    let contact = mailer.contact();
    mailer.send(match notice {
        Notice::EmailChanged { new_email } => mail::email_changed(to, nickname, new_email.as_deref(), contact),
        Notice::PasswordChanged => mail::password_changed(to, nickname, contact),
        Notice::AccountDeleted => mail::account_deleted(to, nickname, contact),
        Notice::PremiumEnded { must_choose } => {
            let link = format!("{}/", state.config.public_url.as_deref().unwrap_or("").trim_end_matches('/'));
            mail::premium_ended(to, nickname, must_choose, &link, contact)
        }
    });
}

/// Tell the account its Premium ended (call once the change is stored and
/// [`db::sync_plan`] ran): whether it must choose what to keep.
pub fn premium_ended(state: &AppState, conn: &Connection, account_id: i64) -> Result<()> {
    let row: Option<(String, Option<String>, bool, bool)> = conn
        .query_row("SELECT nickname, email, must_choose, is_admin FROM accounts WHERE id = ?1", [account_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    if let Some((nickname, Some(to), must_choose, false)) = row {
        notify(state, &to, &nickname, Notice::PremiumEnded { must_choose });
    }
    Ok(())
}

/// An account's nickname and email.
pub fn contact_of(conn: &Connection, account_id: i64) -> Result<Option<(String, Option<String>)>> {
    Ok(conn
        .query_row("SELECT nickname, email FROM accounts WHERE id = ?1", [account_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

/// Check the mail key now and then; tell the admin (Discord) when it stops working.
pub fn spawn_key_check(state: &AppState) {
    if state.mailer.is_none() {
        return;
    }
    let weak = std::sync::Arc::downgrade(&state.0);
    tokio::spawn(async move {
        // Not right at start-up (tests, restarts in a loop).
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let mut failing = false;
        loop {
            let Some(inner) = weak.upgrade() else { break };
            let state = AppState(inner);
            let Some(mailer) = &state.mailer else { break };
            match mailer.check_key().await {
                Ok(()) => {
                    if failing {
                        state.notifier.notice("Eightcount: the Brevo API key works again, emails are sent.");
                    }
                    failing = false;
                }
                Err(e) => {
                    tracing::error!("Brevo API key check failed: {e}");
                    if !failing {
                        state.notifier.notice(&format!(
                            "Eightcount: emails (sign-up confirmation, password reset) are NOT being sent. {e}. \
                             Create a new key in Brevo (Transactional email: All) and set it with \
                             `facade env set arus ARUS_BREVO_API_KEY`."
                        ));
                    }
                    failing = true;
                }
            }
            drop(state);
            tokio::time::sleep(mail::CHECK_EVERY).await;
        }
    });
}
