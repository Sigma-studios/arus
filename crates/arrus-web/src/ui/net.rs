//! Typed calls to the Eightcount API, and the inbox that brings async results back
//! to the UI thread as closures over the [`App`].

use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::rc::Rc;

use arrus_api::{ApiError as ApiErrorBody, Conflict, FILENAME_HEADER};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::App;
use crate::util;
use crate::web::{self, Body};

pub type Handler = Box<dyn FnOnce(&mut App)>;

/// Spawns futures and queues their results for the next frame.
#[derive(Clone)]
pub struct Net {
    inbox: Rc<RefCell<Vec<Handler>>>,
    ctx: egui::Context,
}

impl Net {
    pub fn new(ctx: egui::Context) -> Self {
        Self { inbox: Rc::default(), ctx }
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// Run `fut`; the closure it yields is applied to the app on the UI thread.
    pub fn spawn<F, H>(&self, fut: F)
    where
        F: Future<Output = H> + 'static,
        H: FnOnce(&mut App) + 'static,
    {
        let me = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let h = fut.await;
            me.post(h);
        });
    }

    /// Queue a closure (from a browser callback) for the next frame.
    pub fn post(&self, h: impl FnOnce(&mut App) + 'static) {
        self.inbox.borrow_mut().push(Box::new(h));
        self.ctx.request_repaint();
    }

    pub fn drain(&self) -> Vec<Handler> {
        std::mem::take(&mut *self.inbox.borrow_mut())
    }
}

#[derive(Debug, Clone)]
pub enum ApiError {
    /// 401: not logged in (the app navigates to `/login`).
    Unauthorized,
    /// 409 on save.
    Conflict(Conflict),
    /// 507.
    StorageFull,
    Http { status: u16, message: String, code: Option<String> },
    Network(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "You are not logged in."),
            ApiError::Conflict(c) => write!(f, "{} saved this choreography in the meantime.", c.updated_by),
            ApiError::StorageFull => write!(f, "Storage is full."),
            ApiError::Http { status, message, .. } => {
                if message.is_empty() {
                    write!(f, "Request failed ({status}).")
                } else {
                    write!(f, "{message}")
                }
            }
            ApiError::Network(m) => write!(f, "Could not reach the server: {m}"),
        }
    }
}

impl ApiError {
    /// 404: what was asked for is gone, or not visible to this account.
    pub fn is_not_found(&self) -> bool {
        matches!(self, ApiError::Http { status: 404, .. })
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

async fn call(method: &str, url: &str, body: Body, headers: &[(&str, String)]) -> ApiResult<String> {
    let r = web::fetch(method, url, body, headers).await.map_err(ApiError::Network)?;
    if (200..300).contains(&r.status) {
        return Ok(r.text);
    }
    Err(match r.status {
        401 => ApiError::Unauthorized,
        409 => match serde_json::from_str::<Conflict>(&r.text) {
            Ok(c) => ApiError::Conflict(c),
            Err(_) => http_error(409, &r.text),
        },
        507 => ApiError::StorageFull,
        s => {
            let e = http_error(s, &r.text);
            match &e {
                ApiError::Http { code: Some(c), .. } if c == "storage_full" => ApiError::StorageFull,
                _ => e,
            }
        }
    })
}

fn http_error(status: u16, text: &str) -> ApiError {
    match serde_json::from_str::<ApiErrorBody>(text) {
        Ok(b) => ApiError::Http { status, message: b.error, code: b.code },
        Err(_) => ApiError::Http {
            status,
            message: match status {
                403 => "You are not allowed to do that.".into(),
                404 => "Not found.".into(),
                413 => "That file is too big.".into(),
                _ => text.chars().take(200).collect(),
            },
            code: None,
        },
    }
}

fn parse<T: DeserializeOwned>(text: &str) -> ApiResult<T> {
    serde_json::from_str(text)
        .map_err(|e| ApiError::Http { status: 200, message: format!("Unexpected server response: {e}"), code: None })
}

pub async fn get<T: DeserializeOwned>(url: &str) -> ApiResult<T> {
    parse(&call("GET", url, Body::Empty, &[]).await?)
}

pub async fn send<B: Serialize, T: DeserializeOwned>(method: &str, url: &str, body: &B) -> ApiResult<T> {
    let json = serde_json::to_string(body)
        .map_err(|e| ApiError::Http { status: 0, message: format!("Could not encode: {e}"), code: None })?;
    parse(&call(method, url, Body::Json(json), &[]).await?)
}

/// Send JSON, ignore the (204) response.
pub async fn send_empty_json<B: Serialize>(method: &str, url: &str, body: &B) -> ApiResult<()> {
    let json = serde_json::to_string(body)
        .map_err(|e| ApiError::Http { status: 0, message: format!("Could not encode: {e}"), code: None })?;
    call(method, url, Body::Json(json), &[]).await.map(|_| ())
}

/// A request whose response body is ignored (204s, logout).
pub async fn send_empty(method: &str, url: &str) -> ApiResult<()> {
    call(method, url, Body::Empty, &[]).await.map(|_| ())
}

pub async fn delete_json<T: DeserializeOwned>(url: &str) -> ApiResult<T> {
    parse(&call("DELETE", url, Body::Empty, &[]).await?)
}

/// Upload a browser file as the raw body, with its name in `X-Filename`.
pub async fn upload<T: DeserializeOwned>(method: &str, url: &str, file: web_sys::File, content_type: String) -> ApiResult<T> {
    let name = util::header_filename(&file.name());
    let blob: web_sys::Blob = file.into();
    parse(&call(method, url, Body::Blob(blob, content_type), &[(FILENAME_HEADER, name)]).await?)
}
