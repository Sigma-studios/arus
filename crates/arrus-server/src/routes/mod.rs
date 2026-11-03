//! The router, the CSRF guard, and request helpers shared by the `/api` handlers.

mod choreographies;
mod me;
mod music;
mod teams;
mod transfer;

use arrus_api::CSRF_HEADER;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, FromRequest, Request};
use axum::http::{HeaderMap, Method, StatusCode, header::CONTENT_LENGTH};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::trace::TraceLayer;

use crate::error::{Error, Result};
use crate::{AppState, web};

/// Largest JSON body (choreographies embed background images as base64).
pub const JSON_LIMIT: usize = 64 * 1024 * 1024;

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/me", get(me::me))
        .route("/storage", get(me::storage))
        .route("/teams", get(teams::list).post(teams::create))
        .route("/teams/{id}", patch(teams::update).delete(teams::delete))
        .route("/teams/{id}/members", get(teams::list_members).post(teams::create_member))
        .route("/teams/{id}/import", post(transfer::import))
        .route("/members/{id}", patch(teams::update_member).delete(teams::delete_member))
        .route("/choreographies", get(choreographies::list).post(choreographies::create))
        .route(
            "/choreographies/{id}",
            get(choreographies::get).put(choreographies::save).delete(choreographies::delete),
        )
        .route("/choreographies/{id}/export", get(transfer::export))
        .route("/choreographies/{id}/music", get(music::get).put(music::put).delete(music::delete))
        .fallback(api_not_found)
        .layer(middleware::from_fn(csrf))
        .layer(DefaultBodyLimit::max(JSON_LIMIT));

    Router::new()
        .nest("/api", api)
        .route("/login", get(web::login_page).post(web::login_submit))
        .route("/logout", post(web::logout))
        .route("/healthz", get(web::healthz))
        .fallback(web::fallback)
        // The wasm client shrinks from ~4.7 MB to ~1.7 MB; audio is left alone (already
        // compressed, and seeking uses byte ranges of the original).
        .layer(CompressionLayer::new().compress_when(
            DefaultPredicate::new().and(NotForContentType::const_new("audio/")),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn api_not_found() -> Error {
    Error::not_found()
}

/// Every mutating `/api` request must carry `X-Arus: 1`.
async fn csrf(req: Request, next: Next) -> Response {
    let mutating = matches!(*req.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE);
    if mutating && req.headers().get(CSRF_HEADER).and_then(|v| v.to_str().ok()) != Some("1") {
        return Error::forbidden(format!("missing {CSRF_HEADER} header")).into_response();
    }
    next.run(req).await
}

/// `Json<T>` whose rejections are [`ApiError`](arrus_api::ApiError) bodies.
pub struct ApiJson<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for ApiJson<T> {
    type Rejection = Error;

    async fn from_request(req: Request, state: &S) -> Result<Self> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(rej) if rej.status() == StatusCode::PAYLOAD_TOO_LARGE => Err(Error::too_big(JSON_LIMIT as u64)),
            Err(rej) => Err(Error::new(rej.status(), rej.body_text())),
        }
    }
}

/// Reject early when `Content-Length` announces more than `limit`.
pub fn check_content_length(headers: &HeaderMap, limit: u64) -> Result<()> {
    let len = headers.get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    match len {
        Some(n) if n > limit => Err(Error::too_big(limit)),
        _ => Ok(()),
    }
}

/// Read a whole raw body, failing with 413 `too_big` past `limit`.
pub async fn read_limited(body: Body, limit: u64) -> Result<Bytes> {
    match http_body_util::Limited::new(body, limit as usize).collect().await {
        Ok(c) => Ok(c.to_bytes()),
        Err(e) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => Err(Error::too_big(limit)),
        Err(e) => Err(Error::bad_request(format!("could not read the upload: {e}"))),
    }
}

/// The original file name from `X-Filename` (percent-decoded when it decodes to UTF-8).
pub fn upload_filename(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(arrus_api::FILENAME_HEADER)?;
    let s = match raw.to_str() {
        Ok(s) => percent_encoding::percent_decode_str(s)
            .decode_utf8()
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| s.to_string()),
        Err(_) => String::from_utf8_lossy(raw.as_bytes()).into_owned(),
    };
    // Keep only the last path component, no control characters.
    let name: String = s.rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    let name = name.trim().chars().take(200).collect::<String>();
    (!name.is_empty()).then_some(name)
}
