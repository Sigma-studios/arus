//! `.arrus` / `.aubkp` import, `.arrus` export, and the PDF export.

use arrus_api::{ImportResult, MAX_IMPORT_BYTES};
use arrus_core::format::{self, FormatKey, Imported};
use arrus_core::Choreography;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rusqlite::OptionalExtension;

use super::choreographies::{Stored, insert, readable, summary};
use super::{check_content_length, read_limited, upload_filename};
use crate::activity::Kind;
use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::{AppState, db, storage};

fn key(state: &AppState) -> Result<FormatKey> {
    state.config.format_key.clone().ok_or_else(|| {
        Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "import/export is unavailable: the server has no ArrangeUs format key (set ARUS_FORMAT_KEY)",
        )
    })
}

pub async fn import(
    State(state): State<AppState>,
    auth: Auth,
    Path(team_id): Path<i64>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ImportResult>> {
    auth.user.check_write(team_id)?;
    let key = key(&state)?;
    check_content_length(&headers, MAX_IMPORT_BYTES)?;
    let filename = upload_filename(&headers);
    let bytes = read_limited(body, MAX_IMPORT_BYTES).await?;

    let (choreographies, mut notes) = tokio::task::spawn_blocking(move || parse(&key, &bytes, filename.as_deref()))
        .await
        .map_err(Error::internal)??;
    let by = auth.user.username();
    let account = auth.user.account_id();
    let created = state
        .db(move |s, conn| {
            if !db::team_exists(conn, team_id)? {
                return Err(Error::not_found());
            }
            db::ensure_choreography_room(conn, team_id, choreographies.len() as u32)?;
            let stored = choreographies.iter().map(Stored::new).collect::<Result<Vec<_>>>()?;
            let total: u64 = stored.iter().map(Stored::size).sum();
            storage::ensure_fits(s, conn, 0, total)?;
            let now = db::now();
            let tx = conn.transaction()?;
            let ids = stored.iter().map(|st| insert(&tx, team_id, st, &by, now)).collect::<rusqlite::Result<Vec<_>>>()?;
            tx.commit()?;
            storage::after_change(s, conn)?;
            if let Some(a) = account {
                s.activity.add(a, Kind::Created, ids.len() as i64);
            }
            ids.into_iter().map(|id| summary(conn, id)).collect::<Result<Vec<_>>>()
        })
        .await?;
    if created.is_empty() {
        notes.push("nothing was imported".into());
    }
    Ok(Json(ImportResult { created, notes }))
}

/// Decrypt and pick the choreographies to import, with notes for the user.
fn parse(key: &FormatKey, bytes: &[u8], filename: Option<&str>) -> Result<(Vec<Choreography>, Vec<String>)> {
    let imported = format::read(key, bytes).map_err(|e| {
        Error::with_code(
            StatusCode::BAD_REQUEST,
            "not_arrus",
            match e {
                arrus_core::Error::Json(e) => format!("this ArrangeUs file could not be read: {e}"),
                _ => "this is not an ArrangeUs .arrus or .aubkp file".to_string(),
            },
        )
    })?;
    let mut notes = Vec::new();
    let is_backup = matches!(imported, Imported::Backup(_));
    let all = imported.into_choreographies();
    let mut keep = Vec::new();
    for c in all {
        if is_backup && c.is_deleted == Some(true) {
            notes.push(format!("skipped \"{}\": it is in the backup's trash", c.name));
        } else {
            keep.push(c);
        }
    }
    if is_backup {
        let from = filename.map(|f| format!(" from {f}")).unwrap_or_default();
        notes.insert(0, format!("backup{from}: {} choreograph{} imported", keep.len(), if keep.len() == 1 { "y" } else { "ies" }));
    }
    Ok((keep, notes))
}

pub async fn export(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Response> {
    let key = key(&state)?;
    let json = state
        .db(move |s, conn| {
            readable(conn, &auth.user, id)?;
            if let Some(a) = auth.user.account_id() {
                s.activity.add(a, Kind::Exports, 1);
            }
            let json: Option<String> =
                conn.query_row("SELECT data FROM choreographies WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            json.ok_or_else(Error::not_found)
        })
        .await?;
    let (bytes, name) = tokio::task::spawn_blocking(move || -> Result<_> {
        let c: Choreography = serde_json::from_str(&json).map_err(Error::internal)?;
        let mut iv = [0u8; format::IV_LEN];
        rand::fill(&mut iv);
        Ok((format::write_choreography(&key, &c, iv).map_err(Error::internal)?, c.name))
    })
    .await
    .map_err(Error::internal)??;
    Ok((
        [
            (CONTENT_TYPE, "application/octet-stream".to_string()),
            (CONTENT_DISPOSITION, content_disposition(&name, "arrus")),
            (CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response())
}

/// The choreography as a PDF, one page per formation (no format key needed).
pub async fn pdf(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Response> {
    let json = state
        .db(move |s, conn| {
            readable(conn, &auth.user, id)?;
            if let Some(a) = auth.user.account_id() {
                s.activity.add(a, Kind::Exports, 1);
            }
            let json: Option<String> =
                conn.query_row("SELECT data FROM choreographies WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            json.ok_or_else(Error::not_found)
        })
        .await?;
    let (bytes, name) = tokio::task::spawn_blocking(move || -> Result<_> {
        let c: Choreography = serde_json::from_str(&json).map_err(Error::internal)?;
        Ok((arrus_core::pdf::render(&c), c.name))
    })
    .await
    .map_err(Error::internal)??;
    Ok((
        [
            (CONTENT_TYPE, "application/pdf".to_string()),
            (CONTENT_DISPOSITION, content_disposition(&name, "pdf")),
            (CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response())
}

/// RFC 5987 `attr-char` minus what we encode anyway.
const ATTR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// A file name safe on every OS: no path separators, reserved or control characters.
pub fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let cleaned: String = cleaned.chars().take(120).collect();
    if cleaned.is_empty() { "choreography".into() } else { cleaned }
}

/// `attachment; filename="ascii"; filename*=UTF-8''percent-encoded` (RFC 6266).
pub fn content_disposition(name: &str, ext: &str) -> String {
    let file = format!("{}.{ext}", safe_filename(name));
    let ascii: String = file.chars().map(|c| if c.is_ascii() && c != '%' && c != ';' { c } else { '_' }).collect();
    if ascii == file {
        format!("attachment; filename=\"{file}\"")
    } else {
        format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", utf8_percent_encode(&file, ATTR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions() {
        assert_eq!(content_disposition("Test", "arrus"), "attachment; filename=\"Test.arrus\"");
        assert_eq!(content_disposition("a/b:c\"d", "arrus"), "attachment; filename=\"a_b_c_d.arrus\"");
        assert_eq!(content_disposition("  ", "arrus"), "attachment; filename=\"choreography.arrus\"");
        assert_eq!(
            content_disposition("Café Ä", "arrus"),
            "attachment; filename=\"Caf_ _.arrus\"; filename*=UTF-8''Caf%C3%A9%20%C3%84.arrus"
        );
    }
}
