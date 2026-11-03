//! Errors returned by `/api` handlers, rendered as [`ApiError`] (or [`Conflict`]) JSON.

use arrus_api::{ApiError, Conflict};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub enum Error {
    Api { status: StatusCode, error: String, code: Option<&'static str> },
    Conflict(Conflict),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(status: StatusCode, error: impl Into<String>) -> Self {
        Error::Api { status, error: error.into(), code: None }
    }
    pub fn with_code(status: StatusCode, code: &'static str, error: impl Into<String>) -> Self {
        Error::Api { status, error: error.into(), code: Some(code) }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "not logged in")
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, msg)
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }
    pub fn too_big(limit: u64) -> Self {
        Self::with_code(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_big",
            format!("file too big (max {} MB)", limit / (1024 * 1024)),
        )
    }
    pub fn storage_full() -> Self {
        Self::with_code(
            StatusCode::INSUFFICIENT_STORAGE,
            "storage_full",
            "storage is full: delete some choreographies or music first",
        )
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!("internal error: {e}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
    pub fn status(&self) -> StatusCode {
        match self {
            Error::Api { status, .. } => *status,
            Error::Conflict(_) => StatusCode::CONFLICT,
        }
    }
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Error::Api { code, .. } => *code,
            Error::Conflict(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::internal(format!("sqlite: {e}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::internal(format!("io: {e}"))
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Error::Api { status, error, code } => {
                (status, Json(ApiError { error, code: code.map(str::to_string) })).into_response()
            }
            Error::Conflict(c) => (StatusCode::CONFLICT, Json(c)).into_response(),
        }
    }
}
