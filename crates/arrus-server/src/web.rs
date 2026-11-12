//! HTML side: the login form, logout, and the embedded egui web client.

use std::time::Duration;

use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::Embed;
use serde::Deserialize;

use crate::activity::Kind;
use crate::auth::{self, Auth, User};
use crate::error::{Error, Result};
use crate::routes::{accounts, teams};
use crate::AppState;

/// The trunk build of `arrus-web`. Empty when it has not been built.
#[derive(Embed)]
#[folder = "../arrus-web/dist"]
#[allow_missing = true]
pub struct Assets;

fn redirect(to: &str) -> Response {
    let to = HeaderValue::from_str(to).unwrap_or(HeaderValue::from_static("/"));
    (StatusCode::SEE_OTHER, [(LOCATION, to)]).into_response()
}

/// Where to go after logging in: only an invite link (no open redirects).
fn safe_next(next: Option<&str>) -> Option<String> {
    let next = next?;
    let token = next.strip_prefix("/join/")?;
    (!token.is_empty() && token.len() <= 64 && token.chars().all(|c| c.is_ascii_hexdigit())).then(|| next.to_string())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn with_next(path: &str, next: Option<&str>) -> String {
    match next {
        Some(n) => format!("{path}?next={}", percent_encoding::utf8_percent_encode(n, percent_encoding::NON_ALPHANUMERIC)),
        None => path.to_string(),
    }
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

#[derive(Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    next: Option<String>,
}

pub async fn login_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LoginQuery>,
) -> Result<Response> {
    let next = safe_next(q.next.as_deref());
    if auth::from_headers(&state, &headers).await?.is_some() {
        return Ok(redirect(next.as_deref().unwrap_or("/")));
    }
    let error = if q.error.is_some() {
        r#"<p class="error" role="alert">Wrong nickname, email or password.</p>"#
    } else {
        ""
    };
    let page = LOGIN_HTML
        .replace("{{error}}", error)
        .replace("{{next}}", &esc(next.as_deref().unwrap_or("")))
        .replace("{{signup}}", &esc(&with_next("/signup", next.as_deref())));
    Ok(no_store(Html(page).into_response()))
}

#[derive(Deserialize)]
pub struct LoginForm {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    next: Option<String>,
}

pub async fn login_submit(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Result<Response> {
    let next = safe_next(form.next.as_deref().filter(|n| !n.is_empty()));
    let ok = state
        .db(move |s, conn| {
            let found = auth::check_login(conn, &s.config.admin_password, &form.username, &form.password)?;
            Ok(match found {
                Some(account_id) => {
                    if let Some(id) = account_id {
                        s.activity.add(id, Kind::Logins, 1);
                    }
                    Some(auth::create_session(conn, account_id)?)
                }
                None => None,
            })
        })
        .await?;
    match ok {
        Some(token) => {
            let mut r = redirect(next.as_deref().unwrap_or("/"));
            auth::set_cookie(r.headers_mut(), auth::session_cookie(&state, &token));
            Ok(r)
        }
        None => {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let back = match &next {
                Some(n) => format!("/login?error=1&next={}", percent_encoding::utf8_percent_encode(n, percent_encoding::NON_ALPHANUMERIC)),
                None => "/login?error=1".to_string(),
            };
            Ok(redirect(&back))
        }
    }
}

#[derive(Deserialize, Default)]
pub struct SignupForm {
    #[serde(default)]
    nickname: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    password2: String,
    #[serde(default)]
    next: Option<String>,
}

fn signup_html(form: &SignupForm, next: Option<&str>, error: Option<&str>) -> String {
    let error = error.map(|e| format!(r#"<p class="error" role="alert">{}</p>"#, esc(e))).unwrap_or_default();
    SIGNUP_HTML
        .replace("{{error}}", &error)
        .replace("{{nickname}}", &esc(form.nickname.trim()))
        .replace("{{email}}", &esc(form.email.trim()))
        .replace("{{next}}", &esc(next.unwrap_or("")))
        .replace("{{login}}", &esc(&with_next("/login", next)))
        .replace("{{min}}", &arrus_api::MIN_PASSWORD_LEN.to_string())
}

pub async fn signup_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LoginQuery>,
) -> Result<Response> {
    let next = safe_next(q.next.as_deref());
    if auth::from_headers(&state, &headers).await?.is_some() {
        return Ok(redirect(next.as_deref().unwrap_or("/")));
    }
    Ok(no_store(Html(signup_html(&SignupForm::default(), next.as_deref(), None)).into_response()))
}

pub async fn signup_submit(State(state): State<AppState>, Form(form): Form<SignupForm>) -> Result<Response> {
    let next = safe_next(form.next.as_deref().filter(|n| !n.is_empty()));
    let fail = |form: &SignupForm, msg: &str| {
        no_store((StatusCode::BAD_REQUEST, Html(signup_html(form, next.as_deref(), Some(msg)))).into_response())
    };
    let nickname = match accounts::clean_nickname(&form.nickname) {
        Ok(n) => n,
        Err(e) => return Ok(fail(&form, &message(&e))),
    };
    let email = match accounts::clean_email(&form.email) {
        Ok(Some(e)) => e,
        Ok(None) => return Ok(fail(&form, "Please enter your email.")),
        Err(e) => return Ok(fail(&form, &message(&e))),
    };
    if let Err(e) = accounts::check_new_password(&form.password) {
        return Ok(fail(&form, &message(&e)));
    }
    if form.password != form.password2 {
        return Ok(fail(&form, "The two passwords are different."));
    }
    let password = form.password.clone();
    let created = state
        .db(move |s, conn| {
            let id = accounts::insert_account(conn, &nickname, Some(&email), &password, false)?;
            s.activity.add(id, Kind::Logins, 1);
            Ok(auth::create_session(conn, Some(id))?)
        })
        .await;
    match created {
        Ok(token) => {
            let mut r = redirect(next.as_deref().unwrap_or("/"));
            auth::set_cookie(r.headers_mut(), auth::session_cookie(&state, &token));
            Ok(r)
        }
        Err(e) if e.status() != StatusCode::INTERNAL_SERVER_ERROR => Ok(fail(&form, &message(&e))),
        Err(e) => Err(e),
    }
}

/// An API error as a sentence for an HTML page.
fn message(e: &Error) -> String {
    match e {
        Error::Api { error, .. } => {
            let mut c = error.chars();
            match c.next() {
                Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
                None => String::new(),
            }
        }
        Error::Conflict(_) => "Conflict.".into(),
    }
}

/// A share link: join the team, then open the app on it.
pub async fn join(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>) -> Result<Response> {
    let Some(auth) = auth::from_headers(&state, &headers).await? else {
        let next = safe_next(Some(&format!("/join/{token}")));
        return Ok(redirect(&with_next("/login", next.as_deref())));
    };
    let User::Account(a) = auth.user else {
        return Ok(redirect("/"));
    };
    let joined = state.db(move |_, conn| teams::join(conn, a.id, &token)).await?;
    Ok(match joined {
        Some(team) => redirect(&format!("/#team={team}")),
        None => no_store((StatusCode::NOT_FOUND, Html(INVALID_INVITE_HTML)).into_response()),
    })
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

/// The served client's version (see [`arrus_api::Version`]).
pub async fn version() -> Response {
    no_store(axum::Json(arrus_api::Version { client: client_version().to_owned() }).into_response())
}

fn client_version() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| match Assets::get("index.html") {
        Some(f) => f.metadata.sha256_hash()[..8].iter().map(|b| format!("{b:02x}")).collect(),
        None => "dev".into(),
    })
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
<html lang="en"><head><meta charset="utf-8"><title>Eightcount</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body{font-family:system-ui,sans-serif;background:#0b0b0c;color:#e8e8eb;display:grid;place-items:center;min-height:100vh;margin:0}code{color:#b394ff}</style>
</head><body><div><h1>Eightcount</h1><p>The web client is not built. Run <code>trunk build --release</code> in <code>crates/arrus-web</code> and rebuild the server.</p>
<form method="post" action="/logout"><button>Log out</button></form></div></body></html>
"#;

const LOGIN_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>Eightcount</title>
<style>
  *{box-sizing:border-box}
  body{margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0b0c;color:#e8e8eb;
       font:16px/1.4 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
  main{width:min(380px,92vw);background:#1a1a1d;border:1px solid #2c2c31;border-radius:14px;padding:32px 28px;
       box-shadow:0 10px 40px rgba(0,0,0,.45)}
  h1{margin:0 0 4px;font-size:28px;letter-spacing:.04em;color:#9266f8}
  p.sub{margin:0 0 24px;color:#a0a0a8;font-size:14px}
  label{display:block;font-size:13px;color:#a0a0a8;margin:14px 0 6px}
  input{width:100%;padding:11px 12px;border-radius:8px;border:1px solid #34343a;background:#121214;color:#e8e8eb;font-size:16px}
  input:focus{outline:none;border-color:#9266f8;box-shadow:0 0 0 3px rgba(146,102,248,.25)}
  button{margin-top:22px;width:100%;padding:12px;border:0;border-radius:8px;background:#7c4ff0;color:#fff;
         font-size:16px;font-weight:600;cursor:pointer}
  button:hover{background:#9266f8}
  .error{margin:0 0 8px;padding:10px 12px;border-radius:8px;background:rgba(240,82,82,.12);
         border:1px solid rgba(240,82,82,.45);color:#ffb8b8;font-size:14px}
  p.alt{margin:18px 0 0;text-align:center;color:#a0a0a8;font-size:14px}
  p.hint{margin:6px 0 0;color:#707078;font-size:12px}
  a{color:#b394ff}
</style>
</head>
<body>
<main>
  <h1>Eightcount</h1>
  <p class="sub">Choreographies for your team</p>
  {{error}}
  <form method="post" action="/login">
    <input type="hidden" name="next" value="{{next}}">
    <label for="username">Nickname or email</label>
    <input id="username" name="username" type="text" autocomplete="username" autocapitalize="none"
           autocorrect="off" spellcheck="false" required autofocus>
    <label for="password">Password</label>
    <input id="password" name="password" type="password" autocomplete="current-password" required>
    <button type="submit">Log in</button>
  </form>
  <p class="alt">No account yet? <a href="{{signup}}">Sign up</a></p>
</main>
</body>
</html>
"#;

const SIGNUP_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>Eightcount · sign up</title>
<style>
  *{box-sizing:border-box}
  body{margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0b0c;color:#e8e8eb;
       font:16px/1.4 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
  main{width:min(380px,92vw);background:#1a1a1d;border:1px solid #2c2c31;border-radius:14px;padding:32px 28px;
       box-shadow:0 10px 40px rgba(0,0,0,.45)}
  h1{margin:0 0 4px;font-size:28px;letter-spacing:.04em;color:#9266f8}
  p.sub{margin:0 0 24px;color:#a0a0a8;font-size:14px}
  label{display:block;font-size:13px;color:#a0a0a8;margin:14px 0 6px}
  input{width:100%;padding:11px 12px;border-radius:8px;border:1px solid #34343a;background:#121214;color:#e8e8eb;font-size:16px}
  input:focus{outline:none;border-color:#9266f8;box-shadow:0 0 0 3px rgba(146,102,248,.25)}
  button{margin-top:22px;width:100%;padding:12px;border:0;border-radius:8px;background:#7c4ff0;color:#fff;
         font-size:16px;font-weight:600;cursor:pointer}
  button:hover{background:#9266f8}
  .error{margin:0 0 8px;padding:10px 12px;border-radius:8px;background:rgba(240,82,82,.12);
         border:1px solid rgba(240,82,82,.45);color:#ffb8b8;font-size:14px}
  p.alt{margin:18px 0 0;text-align:center;color:#a0a0a8;font-size:14px}
  p.hint{margin:6px 0 0;color:#707078;font-size:12px}
  a{color:#b394ff}
</style>
</head>
<body>
<main>
  <h1>Eightcount</h1>
  <p class="sub">Create your account</p>
  {{error}}
  <form method="post" action="/signup">
    <input type="hidden" name="next" value="{{next}}">
    <label for="nickname">Nickname</label>
    <input id="nickname" name="nickname" type="text" value="{{nickname}}" autocomplete="username" autocapitalize="none"
           autocorrect="off" spellcheck="false" maxlength="40" pattern="[A-Za-z0-9._\-]+" required autofocus>
    <p class="hint">Letters, digits, '.', '_' and '-'. Others add you to their team with it.</p>
    <label for="email">Email</label>
    <input id="email" name="email" type="email" value="{{email}}" autocomplete="email" required>
    <label for="password">Password</label>
    <input id="password" name="password" type="password" autocomplete="new-password" minlength="{{min}}" required>
    <label for="password2">Password again</label>
    <input id="password2" name="password2" type="password" autocomplete="new-password" minlength="{{min}}" required>
    <button type="submit">Create account</button>
  </form>
  <p class="alt">Already have an account? <a href="{{login}}">Log in</a></p>
</main>
</body>
</html>
"#;

const INVALID_INVITE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>Eightcount · invite</title>
<style>
  *{box-sizing:border-box}
  body{margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0b0c;color:#e8e8eb;
       font:16px/1.4 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
  main{width:min(380px,92vw);background:#1a1a1d;border:1px solid #2c2c31;border-radius:14px;padding:32px 28px;
       box-shadow:0 10px 40px rgba(0,0,0,.45)}
  h1{margin:0 0 4px;font-size:28px;letter-spacing:.04em;color:#9266f8}
  p.sub{margin:0 0 24px;color:#a0a0a8;font-size:14px}
  label{display:block;font-size:13px;color:#a0a0a8;margin:14px 0 6px}
  input{width:100%;padding:11px 12px;border-radius:8px;border:1px solid #34343a;background:#121214;color:#e8e8eb;font-size:16px}
  input:focus{outline:none;border-color:#9266f8;box-shadow:0 0 0 3px rgba(146,102,248,.25)}
  button{margin-top:22px;width:100%;padding:12px;border:0;border-radius:8px;background:#7c4ff0;color:#fff;
         font-size:16px;font-weight:600;cursor:pointer}
  button:hover{background:#9266f8}
  .error{margin:0 0 8px;padding:10px 12px;border-radius:8px;background:rgba(240,82,82,.12);
         border:1px solid rgba(240,82,82,.45);color:#ffb8b8;font-size:14px}
  p.alt{margin:18px 0 0;text-align:center;color:#a0a0a8;font-size:14px}
  p.hint{margin:6px 0 0;color:#707078;font-size:12px}
  a{color:#b394ff}
</style>
</head>
<body>
<main>
  <h1>Eightcount</h1>
  <p class="sub">This invite link does not work anymore. Ask the team's owner for a new one.</p>
  <p class="alt"><a href="/">Open Eightcount</a></p>
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
