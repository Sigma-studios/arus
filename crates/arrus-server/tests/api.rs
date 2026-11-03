//! In-process tests of the whole router.

use arrus_api::*;
use arrus_core::format::{self, FormatKey, Imported};
use arrus_core::Choreography;
use arrus_server::{AppState, Config, app};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples");
const ADMIN_PW: &str = "s3cret admin pw";

struct T {
    app: Router,
    dir: tempfile::TempDir,
}

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Resp {
    fn json<V: DeserializeOwned>(&self) -> V {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {} {}", self.status, String::from_utf8_lossy(&self.body)))
    }
    fn error(&self) -> ApiError {
        self.json()
    }
    fn location(&self) -> &str {
        self.headers.get(header::LOCATION).unwrap().to_str().unwrap()
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn config(dir: &std::path::Path, limit: u64) -> Config {
    Config {
        admin_password: ADMIN_PW.into(),
        port: 0,
        data_dir: dir.to_path_buf(),
        storage_limit: limit,
        format_key: FormatKey::from_env_or_file().ok(),
        insecure_cookies: false,
        dev_autologin: false,
        public_url: None,
    }
}

impl T {
    fn new() -> Self {
        Self::with(|_| {})
    }
    fn with(f: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut c = config(dir.path(), 1024 * 1024 * 1024);
        f(&mut c);
        let app = app(AppState::open(c).unwrap());
        Self { app, dir }
    }

    async fn send(&self, req: Request<Body>) -> Resp {
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        Resp { status, headers, body }
    }

    /// An API request with the CSRF header.
    async fn call(&self, method: Method, uri: &str, cookie: Option<&str>, json: Option<impl Serialize>) -> Resp {
        let mut b = Request::builder().method(method).uri(uri).header(CSRF_HEADER, "1");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, format!("arus_session={c}"));
        }
        let req = match json {
            Some(j) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&j).unwrap()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        self.send(req).await
    }

    async fn get(&self, uri: &str, cookie: &str) -> Resp {
        self.call(Method::GET, uri, Some(cookie), None::<()>).await
    }

    async fn raw(&self, method: Method, uri: &str, cookie: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Resp {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header(CSRF_HEADER, "1")
            .header(header::COOKIE, format!("arus_session={cookie}"));
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        self.send(b.body(Body::from(body)).unwrap()).await
    }

    async fn login_resp(&self, user: &str, pw: &str) -> Resp {
        let body = format!(
            "username={}&password={}",
            percent_encoding::utf8_percent_encode(user, percent_encoding::NON_ALPHANUMERIC),
            percent_encoding::utf8_percent_encode(pw, percent_encoding::NON_ALPHANUMERIC)
        );
        let req = Request::builder()
            .method(Method::POST)
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        self.send(req).await
    }

    async fn login(&self, user: &str, pw: &str) -> String {
        let r = self.login_resp(user, pw).await;
        assert_eq!(r.status, StatusCode::SEE_OTHER);
        assert_eq!(r.location(), "/", "login as {user} failed");
        let cookie = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        cookie.split(';').next().unwrap().strip_prefix("arus_session=").unwrap().to_string()
    }

    async fn admin(&self) -> String {
        self.login("admin", ADMIN_PW).await
    }

    async fn team(&self, admin: &str, name: &str) -> Team {
        let r = self.call(Method::POST, "/api/teams", Some(admin), Some(CreateTeam { name: name.into() })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        r.json()
    }

    async fn member(&self, admin: &str, team: TeamId, username: &str, can_write: bool) -> Member {
        let r = self
            .call(
                Method::POST,
                &format!("/api/teams/{team}/members"),
                Some(admin),
                Some(CreateMember { username: username.into(), can_write }),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        r.json()
    }

    async fn create(&self, cookie: &str, team_id: Option<TeamId>, data: Choreography) -> Resp {
        self.call(Method::POST, "/api/choreographies", Some(cookie), Some(CreateChoreography { team_id, data })).await
    }

    async fn storage(&self, cookie: &str) -> Storage {
        self.get("/api/storage", cookie).await.json()
    }

    fn music_files(&self) -> usize {
        std::fs::read_dir(self.dir.path().join("music")).unwrap().count()
    }
}

fn sample() -> Choreography {
    serde_json::from_str(&std::fs::read_to_string(format!("{SAMPLES}/Test.json")).unwrap()).unwrap()
}

fn key() -> Option<FormatKey> {
    let k = FormatKey::from_env_or_file().ok();
    if k.is_none() {
        eprintln!("skipped: no format key (.arrus-key / ARUS_FORMAT_KEY)");
    }
    k
}

#[tokio::test]
async fn login_logout_and_cookie_flags() {
    let t = T::new();
    // the form
    let r = t.send(Request::get("/login").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK);
    let html = r.text();
    assert!(html.contains("<title>arus</title>"));
    assert!(html.contains(r#"autocomplete="username""#) && html.contains(r#"autocomplete="current-password""#));
    assert!(!html.contains("Wrong username"));
    let r = t.send(Request::get("/login?error=1").body(Body::empty()).unwrap()).await;
    assert!(r.text().contains("Wrong username"));

    // failure
    let started = std::time::Instant::now();
    let r = t.login_resp("admin", "nope").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), "/login?error=1");
    assert!(r.headers.get(header::SET_COOKIE).is_none());
    assert!(started.elapsed() >= std::time::Duration::from_millis(400));
    assert_eq!(t.login_resp("nobody", "x").await.location(), "/login?error=1");

    // success + cookie flags
    let r = t.login_resp("Admin", ADMIN_PW).await;
    assert_eq!(r.location(), "/");
    let cookie = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    assert!(cookie.starts_with("arus_session="), "{cookie}");
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Secure"] {
        assert!(cookie.split("; ").any(|p| p == flag), "{flag} missing: {cookie}");
    }
    let token = cookie.split(';').next().unwrap().strip_prefix("arus_session=").unwrap().to_string();
    assert_eq!(token.len(), 64);

    let me: Me = t.get("/api/me", &token).await.json();
    assert_eq!(me, Me { username: "admin".into(), role: Role::Admin, team: None, can_write: true });

    // logged in: /login redirects, / serves the app
    let r = t
        .send(Request::get("/login").header(header::COOKIE, format!("arus_session={token}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!((r.status, r.location()), (StatusCode::SEE_OTHER, "/"));
    let r = t
        .send(Request::get("/").header(header::COOKIE, format!("arus_session={token}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(r.status, StatusCode::OK);

    // not logged in
    let r = t.send(Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!((r.status, r.location()), (StatusCode::SEE_OTHER, "/login"));
    let r = t.call(Method::GET, "/api/me", None, None::<()>).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    r.error();
    let r = t.get("/api/me", "deadbeef").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // logout ends the session
    let r = t
        .send(
            Request::post("/logout")
                .header(header::COOKIE, format!("arus_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!((r.status, r.location()), (StatusCode::SEE_OTHER, "/login"));
    assert!(r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(t.get("/api/me", &token).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn insecure_cookies_and_dev_autologin() {
    let t = T::with(|c| {
        c.insecure_cookies = true;
        c.dev_autologin = true;
    });
    let r = t.login_resp("admin", ADMIN_PW).await;
    let cookie = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(!cookie.contains("Secure"), "{cookie}");

    let r = t.send(Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK);
    let cookie = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    let token = cookie.split(';').next().unwrap().strip_prefix("arus_session=").unwrap();
    let me: Me = t.get("/api/me", token).await.json();
    assert_eq!(me.role, Role::Admin);

    // without insecure mode autologin is ignored
    let t = T::with(|c| c.dev_autologin = true);
    let r = t.send(Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!((r.status, r.location()), (StatusCode::SEE_OTHER, "/login"));
}

#[tokio::test]
async fn misc_routes() {
    let t = T::new();
    let r = t.send(Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert_eq!((r.status, r.text().as_str()), (StatusCode::OK, "ok"));
    let admin = t.admin().await;
    let r = t.get("/api/nope", &admin).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    r.error();
    // unknown non-API path: app for logged in, 404 otherwise
    assert_eq!(t.get("/some/client/route", &admin).await.status, StatusCode::OK);
    let r = t.send(Request::get("/some/client/route").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn csrf_header_required() {
    let t = T::new();
    let admin = t.admin().await;
    let req = Request::post("/api/teams")
        .header(header::COOKIE, format!("arus_session={admin}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"name":"A"}"#))
        .unwrap();
    let r = t.send(req).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.error().error.contains("X-Arus"));
    let req = Request::delete("/api/teams/1")
        .header(header::COOKIE, format!("arus_session={admin}"))
        .header(CSRF_HEADER, "0")
        .body(Body::empty())
        .unwrap();
    assert_eq!(t.send(req).await.status, StatusCode::FORBIDDEN);
    // GET needs no header
    let req = Request::get("/api/teams").header(header::COOKIE, format!("arus_session={admin}")).body(Body::empty());
    assert_eq!(t.send(req.unwrap()).await.status, StatusCode::OK);
    assert_eq!(t.call(Method::POST, "/api/teams", Some(&admin), Some(CreateTeam { name: "A".into() })).await.status, StatusCode::OK);
}

#[tokio::test]
async fn teams_and_members() {
    let t = T::new();
    let admin = t.admin().await;
    let team = t.team(&admin, "  Crew A ").await;
    assert_eq!(team.name, "Crew A");
    assert_eq!((team.member_count, team.choreography_count, team.bytes_used), (0, 0, 0));
    let r = t.call(Method::POST, "/api/teams", Some(&admin), Some(CreateTeam { name: "crew a".into() })).await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    let m = t.member(&admin, team.id, " Alice ", true).await;
    assert_eq!(m.username, "Alice");
    assert_eq!(m.team_id, team.id);
    let parts: Vec<&str> = m.password.split('-').collect();
    assert_eq!(parts.len(), 3, "{}", m.password);
    assert!(parts[0].chars().all(|c| c.is_ascii_lowercase()) && parts[1].chars().all(|c| c.is_ascii_lowercase()));
    assert!(parts[2].len() == 2 && parts[2].parse::<u32>().is_ok());

    // usernames: unique case-insensitively, not admin, valid chars
    for (name, status) in [
        ("alice", StatusCode::CONFLICT),
        ("ADMIN", StatusCode::CONFLICT),
        ("bad name", StatusCode::BAD_REQUEST),
        ("", StatusCode::BAD_REQUEST),
        (&"x".repeat(41), StatusCode::BAD_REQUEST),
    ] {
        let r = t
            .call(
                Method::POST,
                &format!("/api/teams/{}/members", team.id),
                Some(&admin),
                Some(CreateMember { username: name.into(), can_write: false }),
            )
            .await;
        assert_eq!(r.status, status, "{name}");
    }
    let r = t
        .call(
            Method::POST,
            &format!("/api/teams/{}/members", team.id),
            Some(&admin),
            Some(CreateMember { username: "ALICE".into(), can_write: false }),
        )
        .await;
    assert_eq!(r.error().code.as_deref(), Some("username_taken"));

    // the member logs in (case-insensitive username)
    let alice = t.login("aLiCe", &m.password).await;
    let me: Me = t.get("/api/me", &alice).await.json();
    assert_eq!(me.role, Role::Member);
    assert_eq!(me.team, Some(TeamRef { id: team.id, name: "Crew A".into() }));
    assert!(me.can_write);
    // members cannot manage teams
    assert_eq!(t.get("/api/teams", &alice).await.status, StatusCode::FORBIDDEN);
    let r = t.call(Method::POST, "/api/teams", Some(&alice), Some(CreateTeam { name: "X".into() })).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    let teams: Vec<Team> = t.get("/api/teams", &admin).await.json();
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].member_count, 1);
    let members: Vec<Member> = t.get(&format!("/api/teams/{}/members", team.id), &admin).await.json();
    assert_eq!(members, vec![m.clone()]);

    // rename the team
    let r = t
        .call(Method::PATCH, &format!("/api/teams/{}", team.id), Some(&admin), Some(UpdateTeam { name: "Crew B".into() }))
        .await;
    assert_eq!(r.json::<Team>().name, "Crew B");

    // update: can_write takes effect immediately
    let upd = UpdateMember { can_write: Some(false), username: Some("alice2".into()), ..Default::default() };
    let m2: Member = t.call(Method::PATCH, &format!("/api/members/{}", m.id), Some(&admin), Some(upd)).await.json();
    assert_eq!((m2.username.as_str(), m2.can_write, m2.password.as_str()), ("alice2", false, m.password.as_str()));
    assert!(!t.get("/api/me", &alice).await.json::<Me>().can_write);

    // regenerating the password ends sessions
    let upd = UpdateMember { regenerate_password: true, ..Default::default() };
    let m3: Member = t.call(Method::PATCH, &format!("/api/members/{}", m.id), Some(&admin), Some(upd)).await.json();
    assert_ne!(m3.password, m.password);
    assert_eq!(t.get("/api/me", &alice).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(t.login_resp("alice2", &m.password).await.location(), "/login?error=1");
    // typed with a capital first letter (phone keyboards) still works
    let mut typed = m3.password.clone();
    typed[..1].make_ascii_uppercase();
    let alice = t.login("alice2", &typed).await;

    // deleting the member ends sessions
    let r = t.call(Method::DELETE, &format!("/api/members/{}", m.id), Some(&admin), None::<()>).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.get("/api/me", &alice).await.status, StatusCode::UNAUTHORIZED);
    let r = t.call(Method::DELETE, &format!("/api/members/{}", m.id), Some(&admin), None::<()>).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn permissions() {
    let t = T::new();
    let admin = t.admin().await;
    let a = t.team(&admin, "A").await;
    let b = t.team(&admin, "B").await;
    let reader = t.member(&admin, a.id, "reader", false).await;
    let writer = t.member(&admin, a.id, "writer", true).await;
    let other = t.member(&admin, b.id, "other", true).await;
    let reader = t.login("reader", &reader.password).await;
    let writer = t.login("writer", &writer.password).await;
    let other = t.login("other", &other.password).await;

    // reader cannot create, writer can (team implicit, team_id ignored)
    let r = t.create(&reader, None, sample()).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    r.error();
    let r = t.create(&writer, Some(b.id), sample()).await;
    assert_eq!(r.status, StatusCode::OK);
    let s: ChoreographySummary = r.json();
    assert_eq!(s.team_id, a.id);
    assert_eq!(s.updated_by, "writer");
    let id = s.id;

    // admin must name the team
    assert_eq!(t.create(&admin, None, sample()).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(t.create(&admin, Some(9999), sample()).await.status, StatusCode::NOT_FOUND);
    let in_b: ChoreographySummary = t.create(&admin, Some(b.id), sample()).await.json();
    assert_eq!(in_b.updated_by, "admin");

    // reader reads, cannot change
    assert_eq!(t.get(&format!("/api/choreographies/{id}"), &reader).await.status, StatusCode::OK);
    let list: Vec<ChoreographySummary> = t.get("/api/choreographies", &reader).await.json();
    assert_eq!(list.iter().map(|s| s.id).collect::<Vec<_>>(), vec![id]);
    let save = SaveChoreography { base_revision: 1, data: sample() };
    let r = t.call(Method::PUT, &format!("/api/choreographies/{id}"), Some(&reader), Some(&save)).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = t.call(Method::DELETE, &format!("/api/choreographies/{id}"), Some(&reader), None::<()>).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = t.raw(Method::PUT, &format!("/api/choreographies/{id}/music"), &reader, &[("content-type", "audio/mpeg")], vec![1; 10]).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = t.raw(Method::POST, &format!("/api/teams/{}/import", a.id), &reader, &[], vec![0; 32]).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // another team: 404 everywhere
    for path in ["", "/export", "/music"] {
        let r = t.get(&format!("/api/choreographies/{id}{path}"), &other).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{path}");
    }
    let r = t.call(Method::PUT, &format!("/api/choreographies/{id}"), Some(&other), Some(&save)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = t.call(Method::DELETE, &format!("/api/choreographies/{id}"), Some(&other), None::<()>).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = t.raw(Method::POST, &format!("/api/teams/{}/import", a.id), &other, &[], vec![0; 32]).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(t.get(&format!("/api/choreographies?team={}", a.id), &other).await.status, StatusCode::NOT_FOUND);
    let list: Vec<ChoreographySummary> = t.get("/api/choreographies", &other).await.json();
    assert_eq!(list.iter().map(|s| s.id).collect::<Vec<_>>(), vec![in_b.id]);

    // admin sees everything, must name the team when listing
    assert_eq!(t.get("/api/choreographies", &admin).await.status, StatusCode::BAD_REQUEST);
    let list: Vec<ChoreographySummary> = t.get(&format!("/api/choreographies?team={}", a.id), &admin).await.json();
    assert_eq!(list.len(), 1);
    assert_eq!(t.get(&format!("/api/choreographies/{id}"), &admin).await.status, StatusCode::OK);

    // storage: per-team breakdown for admin only
    let st = t.storage(&reader).await;
    assert!(st.used > 0 && st.teams.is_empty());
    let st = t.storage(&admin).await;
    assert_eq!(st.teams.len(), 2);
    assert_eq!(st.teams.iter().map(|u| u.bytes).sum::<u64>(), st.used);
}

#[tokio::test]
async fn create_get_save_conflict() {
    let t = T::new();
    let admin = t.admin().await;
    let team = t.team(&admin, "A").await;
    let w = t.member(&admin, team.id, "w", true).await;
    let w = t.login("w", &w.password).await;

    let original = sample();
    let before = arrus_server::db::now();
    let s: ChoreographySummary = t.create(&w, None, original.clone()).await.json();
    assert_eq!(s.revision, 1);
    assert_eq!(s.name, original.name);
    assert_eq!(s.formation_count as usize, original.formations.len());
    assert_eq!(s.dancer_count as usize, original.dancer_count());
    assert!(s.size_bytes > 0 && s.music.is_none());

    let doc: ChoreographyDoc = t.get(&format!("/api/choreographies/{}", s.id), &w).await.json();
    assert_eq!(doc.summary, s);
    assert_ne!(doc.data.id, original.id);
    assert!(uuid::Uuid::parse_str(&doc.data.id).is_ok_and(|u| u.get_version_num() == 4));
    assert!(doc.data.creation_timestamp >= before && doc.data.last_edit_timestamp == doc.data.creation_timestamp);
    assert_eq!(doc.data.formations, original.formations);
    assert_eq!(doc.data.extra, original.extra);

    // save
    let mut data = doc.data.clone();
    data.name = "Renamed".into();
    data.id = "client-tampered".into();
    data.formations.truncate(1);
    let saved: Saved = t
        .call(Method::PUT, &format!("/api/choreographies/{}", s.id), Some(&w), Some(SaveChoreography { base_revision: 1, data }))
        .await
        .json();
    assert_eq!(saved.summary.revision, 2);
    assert_eq!(saved.summary.name, "Renamed");
    assert_eq!(saved.summary.formation_count, 1);
    let doc2: ChoreographyDoc = t.get(&format!("/api/choreographies/{}", s.id), &w).await.json();
    assert_eq!(doc2.data.id, doc.data.id, "id is kept");
    assert_eq!(doc2.data.name, "Renamed");
    assert!(doc2.data.last_edit_timestamp >= doc.data.last_edit_timestamp);

    // stale save
    let r = t
        .call(
            Method::PUT,
            &format!("/api/choreographies/{}", s.id),
            Some(&admin),
            Some(SaveChoreography { base_revision: 1, data: doc.data.clone() }),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let c: Conflict = r.json();
    assert_eq!(c.current_revision, 2);
    assert_eq!(c.updated_by, "w");
    assert_eq!(c.updated_at, saved.summary.updated_at);

    // bad JSON body gets an ApiError
    let r = t.raw(Method::PUT, &format!("/api/choreographies/{}", s.id), &w, &[("content-type", "application/json")], b"{".to_vec()).await;
    assert!(r.status.is_client_error());
    r.error();

    // delete
    let r = t.call(Method::DELETE, &format!("/api/choreographies/{}", s.id), Some(&w), None::<()>).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.get(&format!("/api/choreographies/{}", s.id), &w).await.status, StatusCode::NOT_FOUND);
    assert_eq!(t.storage(&w).await.used, 0);
}

#[tokio::test]
async fn storage_quota() {
    let size = serde_json::to_string(&sample()).unwrap().len() as u64;
    let t = T::with(|c| c.storage_limit = size * 2 + size / 2);
    let admin = t.admin().await;
    let team = t.team(&admin, "A").await;
    let a: ChoreographySummary = t.create(&admin, Some(team.id), sample()).await.json();
    let _b: ChoreographySummary = t.create(&admin, Some(team.id), sample()).await.json();
    let r = t.create(&admin, Some(team.id), sample()).await;
    assert_eq!(r.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(r.error().code.as_deref(), Some("storage_full"));

    // growing a save over the limit is refused, shrinking is fine
    let doc: ChoreographyDoc = t.get(&format!("/api/choreographies/{}", a.id), &admin).await.json();
    let mut big = doc.data.clone();
    big.description_text = Some("x".repeat(size as usize));
    let r = t
        .call(Method::PUT, &format!("/api/choreographies/{}", a.id), Some(&admin), Some(SaveChoreography { base_revision: 1, data: big }))
        .await;
    assert_eq!(r.status, StatusCode::INSUFFICIENT_STORAGE);
    let mut small = doc.data.clone();
    small.formations.truncate(1);
    let r = t
        .call(Method::PUT, &format!("/api/choreographies/{}", a.id), Some(&admin), Some(SaveChoreography { base_revision: 1, data: small }))
        .await;
    assert_eq!(r.status, StatusCode::OK);

    // music over the limit
    let r = t
        .raw(Method::PUT, &format!("/api/choreographies/{}/music", a.id), &admin, &[("content-type", "audio/mpeg")], vec![7; size as usize])
        .await;
    assert_eq!(r.status, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(r.error().code.as_deref(), Some("storage_full"));
    assert_eq!(t.music_files(), 0, "rejected upload left no file");

    // deleting frees space
    let r = t.call(Method::DELETE, &format!("/api/choreographies/{}", a.id), Some(&admin), None::<()>).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.create(&admin, Some(team.id), sample()).await.status, StatusCode::OK);
    let st = t.storage(&admin).await;
    assert!(st.used <= st.limit);
}

#[tokio::test]
async fn import_and_export() {
    let Some(key) = key() else { return };
    let t = T::new();
    let admin = t.admin().await;
    let team = t.team(&admin, "A").await;
    let w = t.member(&admin, team.id, "w", true).await;
    let w = t.login("w", &w.password).await;
    let import = format!("/api/teams/{}/import", team.id);

    let blob = std::fs::read(format!("{SAMPLES}/Test.arrus")).unwrap();
    let expected = format::read(&key, &blob).unwrap().into_choreographies().remove(0);
    let r = t.raw(Method::POST, &import, &w, &[(FILENAME_HEADER, "Test.arrus")], blob).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let res: ImportResult = r.json();
    assert_eq!(res.created.len(), 1);
    let s = &res.created[0];
    assert_eq!((s.team_id, s.name.as_str(), s.revision), (team.id, expected.name.as_str(), 1));
    let doc: ChoreographyDoc = t.get(&format!("/api/choreographies/{}", s.id), &w).await.json();
    assert_eq!(doc.data, expected, "import keeps id, timestamps and all fields");

    // export round-trip
    let r = t.get(&format!("/api/choreographies/{}/export", s.id), &w).await;
    assert_eq!(r.status, StatusCode::OK);
    let cd = r.headers.get(header::CONTENT_DISPOSITION).unwrap().to_str().unwrap();
    assert!(cd.starts_with("attachment;") && cd.contains(".arrus\""), "{cd}");
    let back = match format::read(&key, &r.body).unwrap() {
        Imported::Choreography(c) => *c,
        Imported::Backup(_) => panic!("expected a choreography"),
    };
    assert_eq!(back, expected);
    // a fresh IV every time
    let r2 = t.get(&format!("/api/choreographies/{}/export", s.id), &w).await;
    assert_ne!(r.body[..16], r2.body[..16]);

    // a backup: deleted choreographies are skipped and noted
    let mut c1 = sample();
    c1.name = "Kept".into();
    let mut c2 = sample();
    c2.name = "Trashed".into();
    c2.is_deleted = Some(true);
    let backup = serde_json::json!({ "version": 8, "choreographies": [c1, c2], "crews": [], "presets": [], "backgrounds": [] });
    let blob = format::encrypt(&key, &backup.to_string(), [3; 16]);
    let r = t.raw(Method::POST, &import, &w, &[(FILENAME_HEADER, "my%20backup.aubkp")], blob).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let res: ImportResult = r.json();
    assert_eq!(res.created.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Kept"]);
    assert!(res.notes.iter().any(|n| n.contains("Trashed")), "{:?}", res.notes);
    assert!(res.notes.iter().any(|n| n.contains("my backup.aubkp")), "{:?}", res.notes);

    // not an arrus file
    let r = t.raw(Method::POST, &import, &w, &[], b"hello, this is not encrypted at all!".to_vec()).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.error().code.as_deref(), Some("not_arrus"));
    let r = t.raw(Method::POST, &import, &w, &[], vec![0u8; 64]).await;
    assert_eq!(r.error().code.as_deref(), Some("not_arrus"));

    // too big (announced)
    let r = t
        .raw(Method::POST, &import, &w, &[("content-length", &(MAX_IMPORT_BYTES + 1).to_string())], vec![])
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn import_export_without_key() {
    let t = T::with(|c| c.format_key = None);
    let admin = t.admin().await;
    let team = t.team(&admin, "A").await;
    let r = t.raw(Method::POST, &format!("/api/teams/{}/import", team.id), &admin, &[], vec![0; 32]).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.error().error.contains("key"));
    let s: ChoreographySummary = t.create(&admin, Some(team.id), sample()).await.json();
    let r = t.get(&format!("/api/choreographies/{}/export", s.id), &admin).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn music_upload_range_delete() {
    let t = T::new();
    let admin = t.admin().await;
    let team = t.team(&admin, "A").await;
    let s: ChoreographySummary = t.create(&admin, Some(team.id), sample()).await.json();
    let url = format!("/api/choreographies/{}/music", s.id);
    let before = t.storage(&admin).await.used;

    // not audio
    let r = t.raw(Method::PUT, &url, &admin, &[("content-type", "text/plain")], vec![1; 10]).await;
    assert_eq!(r.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    r.error();
    // too big
    let r = t
        .raw(Method::PUT, &url, &admin, &[("content-type", "audio/mpeg"), ("content-length", &(MAX_MUSIC_BYTES + 1).to_string())], vec![])
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.error().code.as_deref(), Some("too_big"));
    // no music yet
    assert_eq!(t.get(&url, &admin).await.status, StatusCode::NOT_FOUND);

    let audio: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let r = t
        .raw(Method::PUT, &url, &admin, &[("content-type", "audio/ogg"), (FILENAME_HEADER, "Chanson%20d%C3%A9t%C3%A9.ogg")], audio.clone())
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let s2: ChoreographySummary = r.json();
    assert_eq!(
        s2.music,
        Some(MusicInfo { filename: "Chanson d\u{e9}t\u{e9}.ogg".into(), content_type: "audio/ogg".into(), bytes: 5000 })
    );
    assert_eq!(t.storage(&admin).await.used, before + 5000);
    assert_eq!(t.music_files(), 1);

    // full GET
    let r = t.get(&url, &admin).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers.get(header::CONTENT_TYPE).unwrap(), "audio/ogg");
    assert_eq!(r.headers.get(header::ACCEPT_RANGES).unwrap(), "bytes");
    assert_eq!(r.body.as_ref(), audio.as_slice());

    // ranges
    let r = t.raw(Method::GET, &url, &admin, &[("range", "bytes=100-199")], vec![]).await;
    assert_eq!(r.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.headers.get(header::CONTENT_RANGE).unwrap(), "bytes 100-199/5000");
    assert_eq!(r.headers.get(header::CONTENT_LENGTH).unwrap(), "100");
    assert_eq!(r.body.as_ref(), &audio[100..200]);
    let r = t.raw(Method::GET, &url, &admin, &[("range", "bytes=4990-")], vec![]).await;
    assert_eq!(r.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.body.as_ref(), &audio[4990..]);
    let r = t.raw(Method::GET, &url, &admin, &[("range", "bytes=-10")], vec![]).await;
    assert_eq!(r.body.as_ref(), &audio[4990..]);
    let r = t.raw(Method::GET, &url, &admin, &[("range", "bytes=6000-")], vec![]).await;
    assert_eq!(r.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(r.headers.get(header::CONTENT_RANGE).unwrap(), "bytes */5000");

    // replacing keeps one file and counts only the new size
    let r = t.raw(Method::PUT, &url, &admin, &[("content-type", "audio/mpeg")], vec![9; 1000]).await;
    assert_eq!(r.json::<ChoreographySummary>().music.unwrap().bytes, 1000);
    assert_eq!(t.music_files(), 1);
    assert_eq!(t.storage(&admin).await.used, before + 1000);

    // delete frees storage and the file
    let r = t.call(Method::DELETE, &url, Some(&admin), None::<()>).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json::<ChoreographySummary>().music.is_none());
    assert_eq!(t.storage(&admin).await.used, before);
    assert_eq!(t.music_files(), 0);

    // deleting the choreography deletes its music
    t.raw(Method::PUT, &url, &admin, &[("content-type", "audio/mpeg")], vec![9; 1000]).await;
    assert_eq!(t.music_files(), 1);
    t.call(Method::DELETE, &format!("/api/choreographies/{}", s.id), Some(&admin), None::<()>).await;
    assert_eq!(t.music_files(), 0);
    assert_eq!(t.storage(&admin).await.used, 0);
}

#[tokio::test]
async fn team_delete_cascades() {
    let t = T::new();
    let admin = t.admin().await;
    let a = t.team(&admin, "A").await;
    let b = t.team(&admin, "B").await;
    let m = t.member(&admin, a.id, "m", true).await;
    let m_session = t.login("m", &m.password).await;
    let s: ChoreographySummary = t.create(&m_session, None, sample()).await.json();
    t.raw(Method::PUT, &format!("/api/choreographies/{}/music", s.id), &m_session, &[("content-type", "audio/mpeg")], vec![1; 2000])
        .await;
    let kept: ChoreographySummary = t.create(&admin, Some(b.id), sample()).await.json();
    assert_eq!(t.music_files(), 1);
    let team_a: Vec<Team> = t.get("/api/teams", &admin).await.json();
    let team_a = team_a.iter().find(|x| x.id == a.id).unwrap();
    assert_eq!((team_a.member_count, team_a.choreography_count), (1, 1));
    assert_eq!(team_a.bytes_used, s.size_bytes + 2000);

    let r = t.call(Method::DELETE, &format!("/api/teams/{}", a.id), Some(&admin), None::<()>).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.get("/api/me", &m_session).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(t.login_resp("m", &m.password).await.location(), "/login?error=1");
    assert_eq!(t.get(&format!("/api/choreographies/{}", s.id), &admin).await.status, StatusCode::NOT_FOUND);
    assert_eq!(t.music_files(), 0);
    let st = t.storage(&admin).await;
    assert_eq!(st.used, kept.size_bytes);
    assert_eq!(st.teams.len(), 1);
    assert_eq!(t.get(&format!("/api/teams/{}/members", a.id), &admin).await.status, StatusCode::NOT_FOUND);
    // the username is free again
    t.member(&admin, b.id, "m", false).await;
}

#[tokio::test]
async fn alert_settings() {
    let t = T::new();
    let admin = t.admin().await;
    let team = t.team(&admin, "Crew").await;
    let m = t.member(&admin, team.id, "bob", true).await;
    let bob = t.login("bob", &m.password).await;

    // admin only
    assert_eq!(t.get("/api/settings/alerts", &bob).await.status, StatusCode::FORBIDDEN);
    let r = t.call(Method::POST, "/api/settings/alerts/test", Some(&bob), None::<()>).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    let s: AlertSettings = t.get("/api/settings/alerts", &admin).await.json();
    assert_eq!(s, AlertSettings::default());
    let r = t.call(Method::POST, "/api/settings/alerts/test", Some(&admin), None::<()>).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json::<ApiError>().code.as_deref(), Some("no_webhook"));

    // only Discord webhooks are accepted (the server never posts elsewhere)
    let bad = UpdateAlertSettings { webhook: Some("https://example.com/hook".into()), mention: None };
    let r = t.call(Method::PUT, "/api/settings/alerts", Some(&admin), Some(bad)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json::<ApiError>().code.as_deref(), Some("invalid_webhook"));
    let bad = UpdateAlertSettings { webhook: None, mention: Some("@someone".into()) };
    let r = t.call(Method::PUT, "/api/settings/alerts", Some(&admin), Some(bad)).await;
    assert_eq!(r.json::<ApiError>().code.as_deref(), Some("invalid_mention"));

    let url = "https://discord.com/api/webhooks/123456789/token-abcd";
    let set = UpdateAlertSettings { webhook: Some(url.into()), mention: Some("@here role:42".into()) };
    let r = t.call(Method::PUT, "/api/settings/alerts", Some(&admin), Some(set)).await;
    assert_eq!(r.status, StatusCode::OK);
    let s: AlertSettings = r.json();
    assert!(s.webhook_set);
    assert_eq!(s.mention, "@here <@&42>");
    let hint = s.webhook_hint.unwrap();
    assert!(!hint.contains("token-abcd") && hint.ends_with("abcd"), "{hint}");
    // the URL itself is never sent back
    assert!(!String::from_utf8_lossy(&t.get("/api/settings/alerts", &admin).await.body).contains("token-abcd"));

    // survives a restart
    let again = app(AppState::open(config(t.dir.path(), 1024 * 1024 * 1024)).unwrap());
    let t2 = T { app: again, dir: tempfile::tempdir().unwrap() };
    let admin2 = t2.admin().await;
    let s2: AlertSettings = t2.get("/api/settings/alerts", &admin2).await.json();
    assert_eq!(s2.mention, "@here <@&42>");
    assert!(s2.webhook_set);

    // a field left out is unchanged; "" clears
    let clear = UpdateAlertSettings { webhook: Some(String::new()), mention: None };
    let s: AlertSettings = t.call(Method::PUT, "/api/settings/alerts", Some(&admin), Some(clear)).await.json();
    assert!(!s.webhook_set);
    assert_eq!(s.mention, "@here <@&42>");
}

/// The next server-sent event's JSON on a live stream (skipping keep-alives).
async fn next_live(body: &mut Body) -> LiveEvent {
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("no live event within 5 s")
            .expect("stream ended")
            .unwrap();
        let Ok(data) = frame.into_data() else { continue };
        let text = String::from_utf8_lossy(&data).into_owned();
        if let Some(json) = text.lines().find_map(|l| l.strip_prefix("data: ")) {
            return serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {json}"));
        }
    }
}

#[tokio::test]
async fn live_patches_merge_and_stream() {
    let t = T::new();
    let admin = t.admin().await;
    let team = t.team(&admin, "A").await;
    let w = t.member(&admin, team.id, "w", true).await;
    let w = t.login("w", &w.password).await;
    let r = t.member(&admin, team.id, "r", false).await;
    let r = t.login("r", &r.password).await;
    let s: ChoreographySummary = t.create(&w, None, sample()).await.json();
    let url = format!("/api/choreographies/{}", s.id);
    let base = serde_json::to_value(t.get(&url, &w).await.json::<ChoreographyDoc>().data).unwrap();

    // The reader follows along live.
    let req = Request::builder()
        .uri(format!("{url}/live?session=reader-1"))
        .header(header::COOKIE, format!("arus_session={r}"))
        // As a browser asks: the stream must still not be compressed (buffered).
        .header(header::ACCEPT_ENCODING, "gzip, br")
        .body(Body::empty())
        .unwrap();
    let res = t.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/event-stream"));
    assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
    let mut live = res.into_body();
    assert_eq!(next_live(&mut live).await, LiveEvent::Hello { revision: 1 });
    match next_live(&mut live).await {
        LiveEvent::Presence { viewers } => {
            assert_eq!(viewers, vec![Viewer { session: "reader-1".into(), username: "r".into(), can_write: false }]);
        }
        e => panic!("{e:?}"),
    }

    // Two writers change different things from the same starting point: both survive.
    let mut a = base.clone();
    a["name"] = "From A".into();
    let mut b = base.clone();
    b["formations"][0]["duration"] = 42.0.into();
    let send = |session: &str, batch: u64, to: &serde_json::Value| PatchChoreography {
        session: session.into(),
        batch,
        ops: patch::diff(&base, to),
    };
    let saved: Saved = t.call(Method::POST, &format!("{url}/patch"), Some(&w), Some(send("a", 1, &a))).await.json();
    assert_eq!(saved.summary.revision, 2);
    assert_eq!(saved.summary.name, "From A");
    let saved: Saved = t.call(Method::POST, &format!("{url}/patch"), Some(&admin), Some(send("b", 7, &b))).await.json();
    assert_eq!(saved.summary.revision, 3);
    assert_eq!(saved.summary.updated_by, "admin");
    let doc: ChoreographyDoc = t.get(&url, &w).await.json();
    assert_eq!(doc.data.name, "From A");
    assert_eq!(doc.data.formations[0].duration, 42.0);

    match next_live(&mut live).await {
        LiveEvent::Patch { revision, session, batch, by, ops } => {
            assert_eq!((revision, session.as_str(), batch, by.as_str()), (2, "a", 1, "w"));
            assert_eq!(ops, patch::diff(&base, &a));
        }
        e => panic!("{e:?}"),
    }
    assert!(matches!(next_live(&mut live).await, LiveEvent::Patch { revision: 3, batch: 7, .. }));

    // Readers cannot patch; changes that break the document are refused.
    let r403 = t.call(Method::POST, &format!("{url}/patch"), Some(&r), Some(send("r", 1, &a))).await;
    assert_eq!(r403.status, StatusCode::FORBIDDEN);
    let mut broken = base.clone();
    broken["formations"] = "nope".into();
    let bad = t.call(Method::POST, &format!("{url}/patch"), Some(&w), Some(send("a", 2, &broken))).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    bad.error();
    let bad_session = t.call(Method::POST, &format!("{url}/patch"), Some(&w), Some(send("", 2, &a))).await;
    assert_eq!(bad_session.status, StatusCode::BAD_REQUEST);
    assert_eq!(t.get(&format!("{url}/live?session=no%20spaces"), &r).await.status, StatusCode::BAD_REQUEST);

    // A whole-document save tells live viewers to reload; deleting tells them it is gone.
    let doc: ChoreographyDoc = t.get(&url, &w).await.json();
    let put = SaveChoreography { base_revision: 3, data: doc.data };
    assert_eq!(t.call(Method::PUT, &url, Some(&w), Some(put)).await.status, StatusCode::OK);
    assert_eq!(next_live(&mut live).await, LiveEvent::Hello { revision: 4 });
    assert_eq!(t.call(Method::DELETE, &url, Some(&w), None::<()>).await.status, StatusCode::NO_CONTENT);
    assert_eq!(next_live(&mut live).await, LiveEvent::Deleted);
}
