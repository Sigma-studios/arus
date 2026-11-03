//! Admin settings: where storage alerts go.

use arrus_api::{AlertSettings, UpdateAlertSettings};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use super::ApiJson;
use crate::alerts;
use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::{AppState, storage};

pub async fn get_alerts(State(state): State<AppState>, auth: Auth) -> Result<Json<AlertSettings>> {
    auth.user.require_admin()?;
    Ok(Json(state.notifier.discord().settings()))
}

pub async fn update_alerts(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(body): ApiJson<UpdateAlertSettings>,
) -> Result<Json<AlertSettings>> {
    auth.user.require_admin()?;
    let mut d = state.notifier.discord();
    if let Some(w) = body.webhook {
        d.webhook = if w.trim().is_empty() {
            None
        } else {
            Some(alerts::parse_webhook(&w).ok_or_else(|| {
                Error::with_code(
                    StatusCode::BAD_REQUEST,
                    "invalid_webhook",
                    "That is not a Discord webhook URL (https://discord.com/api/webhooks/…).",
                )
            })?)
        };
    }
    if let Some(m) = body.mention {
        let m = alerts::parse_mention(&m).ok_or_else(|| {
            Error::with_code(
                StatusCode::BAD_REQUEST,
                "invalid_mention",
                "Use @here, @everyone, user:<id> or role:<id> (ids from Discord's developer mode).",
            )
        })?;
        d.mention = (!m.is_empty()).then_some(m);
    }
    let saved = d.clone();
    state.db(move |_, conn| Ok(saved.save(conn)?)).await?;
    state.notifier.set_discord(d.clone());
    Ok(Json(d.settings()))
}

pub async fn test_alerts(State(state): State<AppState>, auth: Auth) -> Result<StatusCode> {
    auth.user.require_admin()?;
    if state.notifier.discord().webhook.is_none() {
        return Err(Error::with_code(StatusCode::BAD_REQUEST, "no_webhook", "Set a Discord webhook first."));
    }
    let used = state.db(|_, conn| Ok(storage::usage(conn)?)).await?;
    state
        .notifier
        .test(used, state.config.storage_limit)
        .await
        .map_err(|e| Error::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

