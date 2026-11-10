# arus

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
| `ARUS_PUBLIC_URL` | | linked from alerts |
| `ARUS_INSECURE_COOKIES` | | `1` for plain-http local development |

## Windows app

`desktop/` is a small Flutter app that opens the hosted arus
(`https://arus.online-app.xyz`) in its own window, rendered by Edge WebView2.
Everything else (accounts, choreographies, music) stays on the server; the
window keeps its own login under `%APPDATA%`. `arus.exe --url <url>` (or
`ARUS_URL`) points it at another server, e.g. a local one.

Building needs Flutter, Visual Studio (Build Tools) with the C++ workload, and
Windows Developer Mode (Flutter plugins use symlinks):

```sh
cd desktop
flutter build windows --release
```

The app is the whole `build/windows/x64/runner/Release` folder (exe, DLLs, `data/`,
the MSVC runtime included); zip it to hand it out. It needs the WebView2 Runtime,
which Windows 10/11 already ship.

## Deploying (facade)

The `Dockerfile` builds the client and the server into one image:

```sh
facade app create arus --git https://github.com/Sigma-studios/arus \
  --port 8080 --domain arus.online-app.xyz --volume data:/data:6G \
  --health "arrus-server healthcheck"
facade env set arus ARUS_ADMIN_PASSWORD     # prompts; sensitive
facade env set arus ARUS_FORMAT_KEY
facade env set arus --public ARUS_PUBLIC_URL=https://arus.online-app.xyz
facade deploy arus
```

The volume is capped at 6 GiB, a backstop above the app's own 5 GiB limit, so the
database always has room to write. facade backs it up daily.
