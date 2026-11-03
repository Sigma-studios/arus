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

* One **admin** (`admin`, password from `ARUS_ADMIN_PASSWORD`) manages teams and
  members and can do everything else too.
* **Team members** get a login with a generated password (`tiger-maple-42`) that
  only the admin can see, change or regenerate. A person belongs to one team.
* Every member reads all of their team's choreographies; members with **write**
  access can create, edit, import, delete and upload music for all of them.
* Two people editing the same choreography: the second to save is offered to save
  their version as a new choreography.
* Phones and tablets are view-only.

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
