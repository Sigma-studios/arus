//! One optional audio file per choreography, stored under `<data>/music/`.

use std::io::SeekFrom;

use arrus_api::{ChoreographySummary, LiveEvent, MAX_MUSIC_BYTES};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_NONE_MATCH, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use rusqlite::params;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::choreographies::{readable, summary, writable};
use super::{check_content_length, upload_filename};
use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::{AppState, storage};

#[derive(Debug, PartialEq, Eq)]
pub enum Range {
    Full,
    /// Inclusive byte range.
    Partial(u64, u64),
    Unsatisfiable,
}

/// Parse a `Range` header for a resource of `len` bytes. Only single ranges are
/// honoured; anything we do not understand serves the whole file (allowed by RFC 9110).
pub fn parse_range(header: Option<&str>, len: u64) -> Range {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else { return Range::Full };
    if spec.contains(',') {
        return Range::Full;
    }
    let Some((a, b)) = spec.trim().split_once('-') else { return Range::Full };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        // suffix: last N bytes
        let Ok(n) = b.parse::<u64>() else { return Range::Full };
        if n == 0 || len == 0 {
            return Range::Unsatisfiable;
        }
        return Range::Partial(len.saturating_sub(n), len - 1);
    }
    let Ok(start) = a.parse::<u64>() else { return Range::Full };
    if start >= len {
        return Range::Unsatisfiable;
    }
    let end = if b.is_empty() {
        len - 1
    } else {
        match b.parse::<u64>() {
            Ok(e) if e >= start => e.min(len - 1),
            _ => return Range::Full,
        }
    };
    Range::Partial(start, end)
}

pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response> {
    let (file, content_type) = state
        .db(move |_, conn| {
            readable(conn, &auth.user, id)?;
            let row: (Option<String>, Option<String>) = conn.query_row(
                "SELECT music_file, music_content_type FROM choreographies WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            match row {
                (Some(f), ct) => Ok((f, ct.unwrap_or_else(|| "application/octet-stream".into()))),
                (None, _) => Err(Error::new(StatusCode::NOT_FOUND, "this choreography has no music")),
            }
        })
        .await?;
    let etag = format!("\"{file}\"");
    let mut f = tokio::fs::File::open(state.music_dir.join(&file)).await.map_err(|e| {
        tracing::error!("music file {file} missing: {e}");
        Error::new(StatusCode::NOT_FOUND, "the music file is missing")
    })?;
    let len = f.metadata().await?.len();

    let mut h = HeaderMap::new();
    h.insert(CONTENT_TYPE, HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("audio/mpeg")));
    h.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(ETAG, v);
    }
    let if_none_match = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok());
    if if_none_match.is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*")) {
        return Ok((StatusCode::NOT_MODIFIED, h).into_response());
    }

    let range = parse_range(headers.get(RANGE).and_then(|v| v.to_str().ok()), len);
    let (status, start, count) = match range {
        Range::Full => (StatusCode::OK, 0, len),
        Range::Partial(s, e) => {
            h.insert(CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {s}-{e}/{len}")).expect("ascii"));
            (StatusCode::PARTIAL_CONTENT, s, e - s + 1)
        }
        Range::Unsatisfiable => {
            h.insert(CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{len}")).expect("ascii"));
            return Ok((StatusCode::RANGE_NOT_SATISFIABLE, h).into_response());
        }
    };
    if start > 0 {
        f.seek(SeekFrom::Start(start)).await?;
    }
    h.insert(CONTENT_LENGTH, HeaderValue::from(count));
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(f.take(count)));
    Ok((status, h, body).into_response())
}

pub async fn put(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ChoreographySummary>> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !content_type.to_ascii_lowercase().starts_with("audio/") {
        return Err(Error::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "only audio files can be used as music"));
    }
    check_content_length(&headers, MAX_MUSIC_BYTES)?;
    let filename = upload_filename(&headers).unwrap_or_else(|| "music".into());

    // Permission (and an early quota check when the size is announced) before reading.
    let announced: Option<u64> =
        headers.get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let user = auth.user.clone();
    state
        .db(move |s, conn| {
            let meta = writable(conn, &user, id)?;
            if let Some(n) = announced {
                storage::ensure_fits(s, conn, meta.music_bytes, n)?;
            }
            Ok(())
        })
        .await?;

    let name = format!("{id}-{}.audio", uuid::Uuid::new_v4().simple());
    let path = state.music_dir.join(&name);
    let written = match stream_to_file(body, &path).await {
        Ok(n) => n,
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e);
        }
    };

    let path2 = path.clone();
    let res = state
        .db(move |s, conn| {
            let meta = writable(conn, &auth.user, id)?;
            storage::ensure_fits(s, conn, meta.music_bytes, written)?;
            conn.execute(
                "UPDATE choreographies SET music_file = ?1, music_filename = ?2, music_content_type = ?3, \
                 music_bytes = ?4 WHERE id = ?5",
                params![name, filename, content_type, written as i64, id],
            )?;
            storage::remove_music_files(s, meta.music_file);
            storage::after_change(s, conn)?;
            summary(conn, id)
        })
        .await;
    if res.is_err() {
        let _ = tokio::fs::remove_file(&path2).await;
    }
    let summary = res?;
    state.hub.send(id, &LiveEvent::Summary { summary: summary.clone() });
    Ok(Json(summary))
}

async fn stream_to_file(mut body: Body, path: &std::path::Path) -> Result<u64> {
    let mut file = tokio::fs::File::create(path).await?;
    let mut total: u64 = 0;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| Error::bad_request(format!("could not read the upload: {e}")))?;
        if let Ok(data) = frame.into_data() {
            total += data.len() as u64;
            if total > MAX_MUSIC_BYTES {
                return Err(Error::too_big(MAX_MUSIC_BYTES));
            }
            file.write_all(&data).await?;
        }
    }
    if total == 0 {
        return Err(Error::bad_request("the music file is empty"));
    }
    file.flush().await?;
    file.sync_all().await?;
    Ok(total)
}

pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<i64>) -> Result<Json<ChoreographySummary>> {
    state
        .db(move |s, conn| {
            let meta = writable(conn, &auth.user, id)?;
            conn.execute(
                "UPDATE choreographies SET music_file = NULL, music_filename = NULL, music_content_type = NULL, \
                 music_bytes = NULL WHERE id = ?1",
                [id],
            )?;
            storage::remove_music_files(s, meta.music_file);
            storage::after_change(s, conn)?;
            let summary = summary(conn, id)?;
            s.hub.send(id, &LiveEvent::Summary { summary: summary.clone() });
            Ok(Json(summary))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=0-9"), 100), Range::Partial(0, 9));
        assert_eq!(parse_range(Some("bytes=10-"), 100), Range::Partial(10, 99));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Range::Partial(90, 99));
        assert_eq!(parse_range(Some("bytes=-500"), 100), Range::Partial(0, 99));
        assert_eq!(parse_range(Some("bytes=50-500"), 100), Range::Partial(50, 99));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), 100), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=9-3"), 100), Range::Full);
        assert_eq!(parse_range(Some("items=0-1"), 100), Range::Full);
    }
}
