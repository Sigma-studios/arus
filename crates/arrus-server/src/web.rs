//! HTML side: the login form, logout, and the embedded egui web client.

use std::time::Duration;

use axum::Form;
use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::Embed;
use serde::Deserialize;

use crate::auth::{self, Auth};
use crate::error::Result;
use crate::AppState;

/// The trunk build of `arrus-web`. Empty when it has not been built.
#[derive(Embed)]
#[folder = "../arrus-web/dist"]
#[allow_missing = true]
pub struct Assets;

fn redirect(to: &'static str) -> Response {
    (StatusCode::SEE_OTHER, [(LOCATION, HeaderValue::from_static(to))]).into_response()
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

#[derive(Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    error: Option<String>,
}

pub async fn login_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LoginQuery>,
) -> Result<Response> {
    if auth::from_headers(&state, &headers).await?.is_some() {
        return Ok(redirect("/"));
    }
    let error = if q.error.is_some() {
        r#"<p class="error" role="alert">Wrong username or password.</p>"#
    } else {
        ""
    };
    Ok(no_store(Html(LOGIN_HTML.replace("{{error}}", error)).into_response()))
}

#[derive(Deserialize)]
pub struct LoginForm {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

pub async fn login_submit(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Result<Response> {
    let ok = state
        .db(move |s, conn| {
            let found = auth::check_login(conn, &s.config.admin_password, &form.username, &form.password)?;
            Ok(match found {
                Some(member_id) => Some(auth::create_session(conn, member_id)?),
                None => None,
            })
        })
        .await?;
    match ok {
        Some(token) => {
            let mut r = redirect("/");
            auth::set_cookie(r.headers_mut(), auth::session_cookie(&state, &token));
            Ok(r)
        }
        None => {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(redirect("/login?error=1"))
        }
    }
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    if let Some(token) = auth::session_token(&headers) {
        state
            .db(move |_, conn| {
                conn.execute("DELETE FROM sessions WHERE token_hash = ?1", [auth::hash_token(&token)])?;
                Ok(())
            })
            .await?;
    }
    let mut r = redirect("/login");
    auth::set_cookie(r.headers_mut(), auth::clear_cookie(&state));
    Ok(r)
}

pub async fn healthz() -> &'static str {
    "ok"
}

/// The client's `index.html`, or a placeholder when the client is not built.
fn index_response() -> Response {
    let r = match Assets::get("index.html") {
        Some(f) => ([(CONTENT_TYPE, "text/html; charset=utf-8")], f.data.into_owned()).into_response(),
        None => Html(NOT_BUILT_HTML).into_response(),
    };
    no_store(r)
}

/// Trunk names hashed files like `arrus-web-1a2b3c4d5e6f7a8b_bg.wasm`.
fn is_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    stem.split(['-', '_']).any(|p| p.len() >= 16 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Everything that is not `/api`, `/login`, `/logout`, `/healthz`.
pub async fn fallback(State(state): State<AppState>, uri: Uri, headers: HeaderMap) -> Result<Response> {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() || path == "index.html" {
        return index(&state, &headers).await;
    }
    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let cache = if is_hashed(path) { "public, max-age=31536000, immutable" } else { "no-cache" };
        return Ok((
            [(CONTENT_TYPE, mime.as_ref().to_string()), (CACHE_CONTROL, cache.to_string())],
            file.data.into_owned(),
        )
            .into_response());
    }
    // Client-side routes: the app for logged-in users, 404 for others.
    if auth::from_headers(&state, &headers).await?.is_some() {
        return Ok(index_response());
    }
    Ok((StatusCode::NOT_FOUND, "not found").into_response())
}

async fn index(state: &AppState, headers: &HeaderMap) -> Result<Response> {
    match auth::from_headers(state, headers).await? {
        Some(Auth { refreshed, token, .. }) => {
            let mut r = index_response();
            if refreshed {
                auth::set_cookie(r.headers_mut(), auth::session_cookie(state, &token));
            }
            Ok(r)
        }
        None if state.config.dev_autologin && state.config.insecure_cookies => {
            let token = state.db(|_, conn| Ok(auth::create_session(conn, None)?)).await?;
            tracing::warn!("dev auto-login: issued an admin session");
            let mut r = index_response();
            auth::set_cookie(r.headers_mut(), auth::session_cookie(state, &token));
            Ok(r)
        }
        None => Ok(redirect("/login")),
    }
}

const NOT_BUILT_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>arus</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body{font-family:system-ui,sans-serif;background:#111318;color:#e6e6e6;display:grid;place-items:center;min-height:100vh;margin:0}code{color:#ff6b8b}</style>
</head><body><div><h1>arus</h1><p>The web client is not built. Run <code>trunk build --release</code> in <code>crates/arrus-web</code> and rebuild the server.</p>
<form method="post" action="/logout"><button>Log out</button></form></div></body></html>
"#;

const LOGIN_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>arus</title>
<style>
  *{box-sizing:border-box}
  body{margin:0;min-height:100vh;display:grid;place-items:center;background:#111318;color:#e6e6e6;
       font:16px/1.4 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
  main{width:min(360px,92vw);background:#1b1e26;border:1px solid #2a2f3a;border-radius:14px;padding:32px 28px;
       box-shadow:0 10px 40px rgba(0,0,0,.35)}
  h1{margin:0 0 4px;font-size:28px;letter-spacing:.04em;color:#ff6b8b}
  p.sub{margin:0 0 24px;color:#9aa3b2;font-size:14px}
  label{display:block;font-size:13px;color:#9aa3b2;margin:14px 0 6px}
  input{width:100%;padding:11px 12px;border-radius:8px;border:1px solid #333a47;background:#12151b;color:#e6e6e6;font-size:16px}
  input:focus{outline:none;border-color:#ff6b8b;box-shadow:0 0 0 3px rgba(255,107,139,.2)}
  button{margin-top:22px;width:100%;padding:12px;border:0;border-radius:8px;background:#e92e67;color:#fff;
         font-size:16px;font-weight:600;cursor:pointer}
  button:hover{background:#ff4a7d}
  .error{margin:0 0 8px;padding:10px 12px;border-radius:8px;background:rgba(233,46,103,.12);
         border:1px solid rgba(233,46,103,.45);color:#ffb3c4;font-size:14px}
</style>
</head>
<body>
<main>
  <h1>arus</h1>
  <p class="sub">Choreographies for your team</p>
  {{error}}
  <form method="post" action="/login">
    <label for="username">Username</label>
    <input id="username" name="username" type="text" autocomplete="username" autocapitalize="none"
           autocorrect="off" spellcheck="false" required autofocus>
    <label for="password">Password</label>
    <input id="password" name="password" type="password" autocomplete="current-password" required>
    <button type="submit">Log in</button>
  </form>
</main>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn hashed_names() {
        assert!(super::is_hashed("arrus-web-1a2b3c4d5e6f7a8b_bg.wasm"));
        assert!(super::is_hashed("arrus-web-1a2b3c4d5e6f7a8b.js"));
        assert!(!super::is_hashed("favicon.ico"));
        assert!(!super::is_hashed("manifest.json"));
    }
}
