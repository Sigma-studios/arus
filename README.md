# Eightcount

(Formerly "arus"; internal names such as the crates, the repository, the
Facade app and `arus.db` keep the old name.)

A web app to view and edit dance choreographies made with the
[ArrangeUs](https://play.google.com/store/apps/details?id=com.AndriyStefanchuk.ArrangeUs.main)
phone app: import `.arrus` files and `.aubkp` backups, watch formations animate
(with the music), edit dancers, formations and props, and export back to `.arrus`
for the phone.

Written in Rust: an [egui](https://github.com/emilk/egui) client compiled to
WebAssembly, served by an [axum](https://github.com/tokio-rs/axum) server that
embeds it.

## Accounts

* Anyone can **sign up** at `/signup` with a nickname (unique), an email and a
  password, and logs in with either the nickname or the email.
* A **free** account can create 1 team, holding up to 2 choreographies.
  **Premium** accounts have no limits. Teams made by the admin have none either.
* A team's **owner** adds people by nickname or with a **share link**
  (`/join/…`), as **co-owner** (edits the choreographies too) or **guest**
  (views them). An account can be in several teams and switches between them.
* The **admin** (`admin`, password from `ARUS_ADMIN_PASSWORD`) sees every team
  and every account (Accounts screen): search, premium and admin rights, emails,
  passwords. Accounts given admin rights can do the same.
* Passwords people choose are stored hashed (argon2): nobody can read them.
  Passwords the server generates (`tiger-maple-42`) or the admin sets stay
  visible to the admin until the person changes theirs.
* Two people editing the same choreography: changes merge live; a whole-document
  save that conflicts is offered as a new choreography.
* Live editing runs over one WebSocket per open editor (`/api/choreographies/{id}/ws`):
  changes, presence, and dancers being dragged (streamed ~10×/s, never stored).
  Where a proxy blocks WebSockets the editor falls back to server-sent events
  plus POSTs. Live changes are kept in memory and written to SQLite at most every
  2 s, before any other request reads the database, and on shutdown.
* Phones and tablets are view-only.

The accounts replaced the earlier per-team members: on first start the server
turns every member into an account (same nickname, password and sessions) in
their team, as co-owner if they could write and guest otherwise.

## Storage

Everything stored (choreographies, their embedded background images, uploaded
music) counts towards one app-wide limit of 5 GiB. The admin sets a Discord webhook
in Teams & members → Storage alerts (with a test button); it gets a message at 80 %
and 95 % and when a save or upload is refused because storage is full.

## Crates

| crate | what |
|---|---|
| `arrus-core` | the `.arrus`/`.aubkp` container, typed model of the JSON, playback timing |
| `arrus-api` | request/response types shared by server and client |
| `arrus-server` | HTTP server: login, teams, storage, alerts; embeds the web client |
| `arrus-web` | the egui client (trunk) |

`docs/arrus-format.md` documents the file format and its full JSON schema.
`tools/arrus.py` decodes a file on the command line.

## The format key

ArrangeUs encrypts its files with a fixed key compiled into the app. It is not in
this repository. Put it in a `.arrus-key` file at the repository root (ignored by
git) or in `ARUS_FORMAT_KEY`; without it, import and export are disabled.

## Running locally

```sh
# the web client (rebuilds on change, proxies the API to :8080)
cd crates/arrus-web && trunk serve

# the server
ARUS_ADMIN_PASSWORD=dev ARUS_INSECURE_COOKIES=1 ARUS_DATA_DIR=./data \
  cargo run -p arrus-server
```

`cargo test` runs the tests; the file round-trip tests need the format key.

## Configuration

| variable | default | |
|---|---|---|
| `ARUS_ADMIN_PASSWORD` | (required) | the admin's password |
| `ARUS_FORMAT_KEY` | `.arrus-key` | ArrangeUs file key (32 characters) |
| `ARUS_DATA_DIR` | `./data` | SQLite database and music files |
| `ARUS_PORT` | `8080` | |
| `ARUS_STORAGE_LIMIT_BYTES` | 5 GiB | |
| `ARUS_PUBLIC_URL` | | linked from alerts and emails (needed for emails) |
| `ARUS_BREVO_API_KEY` | | turns emails on (with `ARUS_MAIL_FROM`): confirm the address at sign-up, reset a forgotten password; sent through [Brevo](https://www.brevo.com)'s API |
| `ARUS_MAIL_FROM` | | sender address, e.g. `no-reply@online-app.xyz` (its domain verified in Brevo) |
| `ARUS_MAIL_FROM_NAME` | `Eightcount` | sender name |
| `ARUS_MAIL_REPLY_TO` | | where replies go (the sender address is not read) |
| `ARUS_INSECURE_COOKIES` | | `1` for plain-http local development |

## Apps (Windows, macOS, Linux, Android, iOS)

`app/` is a small Flutter app that opens the hosted Eightcount
(`https://arus.online-app.xyz`) in its own window. Everything else (accounts,
choreographies, music) stays on the server, so web updates reach the apps
without a new install. `--url <url>` (or `ARUS_URL`, desktop) points it at
another server.

| System | Web engine | Notes |
|---|---|---|
| Windows | Edge WebView2 (`lib/windows_view.dart`) | login kept under `%APPDATA%` |
| Android, iOS, macOS | system webview (`lib/webkit_view.dart`) | exports and (macOS) uploads go through the `ArusApp` bridge, see `crates/arrus-web/src/web.rs` |
| Linux | WebKitGTK, in the native runner (`linux/runner/my_application.cc`) | Flutter has no Linux webview; needs `libwebkit2gtk-4.1` |

The site hides its Download page inside the apps (`web::in_app`).

**Releases** are built by GitHub Actions (`.github/workflows/apps.yml`): push a
tag `app-v<version>` (e.g. `app-v1.1.0`) and it publishes a release with
`eightcount-windows.zip`, `eightcount-macos.zip`, `eightcount-linux-x64.tar.gz` and
`eightcount-android.apk`, which the Download page links to
(`releases/latest/download/<file>`). Every push to `main` touching `app/`
builds all platforms as a check. The Android release key lives outside the
repo; CI gets it from the `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` secrets. iOS is published
through the App Store (bundle ID `xyz.onlineapp.eightcount`); set its link in
`crates/arrus-web/src/ui/download.rs` (`APP_STORE`).

Building locally needs Flutter plus the platform's tools (Visual Studio C++
workload and Windows Developer Mode on Windows, Xcode for macOS/iOS, the Android
SDK, or `libgtk-3-dev libwebkit2gtk-4.1-dev` on Linux):

```sh
cd app
flutter build windows --release   # or macos, linux, apk, ios
```

Icons: `python3 tools/app_icon.py`, then `dart run flutter_launcher_icons` in `app/`.

## Deploying (facade)

The `Dockerfile` builds the client and the server into one image:

```sh
facade app create arus --git https://github.com/Sigma-studios/arus \
  --port 8080 --domain arus.online-app.xyz --volume data:/data:6G \
  --health "arrus-server healthcheck"
facade env set arus ARUS_ADMIN_PASSWORD     # prompts; sensitive
facade env set arus ARUS_FORMAT_KEY
facade env set arus --public ARUS_PUBLIC_URL=https://arus.online-app.xyz
facade env set arus ARUS_BREVO_API_KEY        # prompts; sensitive (optional: emails)
facade env set arus --public ARUS_MAIL_FROM=no-reply@online-app.xyz
facade deploy arus
```

The volume is capped at 6 GiB, a backstop above the app's own 5 GiB limit, so the
database always has room to write. facade backs it up daily.
