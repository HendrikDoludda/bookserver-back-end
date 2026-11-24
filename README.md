# Bookserver Backend

A self-hosted server for your own book, manga and comic library, written in Rust. It scans your
folders, catalogs the files in SQLite, extracts their covers and serves everything over an HTTP
API to the [bookserver app](https://github.com/HendrikDoludda/bookserver-front-end).

Not released yet: [what works today](docs/project-state.md) ·
[what comes next](docs/build-checklist.md).

## Quick start

### Download

Each version has a ready-to-run server for macOS (Intel and Apple Silicon), Windows and Linux
(x64) on the [Releases](https://github.com/HendrikDoludda/bookserver-back-end/releases) page —
no Rust needed, PDFium included. The `README.txt` in each download says how to start it on your
system.

### From source

Needs Rust 1.96 or newer. PDF covers also need the PDFium library, as step 1 of
[local.md](docs/run-deploy/local.md#run) describes; without it, PDFs are listed without covers.

```sh
git clone https://github.com/HendrikDoludda/bookserver-back-end.git
cd bookserver-back-end
cargo run
```

### First steps

1. Open http://localhost:8080/health — it answers `pong`.
2. Copy the setup code from the server's start log; it is there while no account exists.
3. Sign up with that code in the [bookserver app](https://github.com/HendrikDoludda/bookserver-front-end),
   connected to `http://localhost:8080`. The first account becomes the admin.
4. Add your book folders in the app. Each one is scanned at once, and later changes are picked
   up as they happen.

Port, network address and mail: [local.md](docs/run-deploy/local.md). Phones and other devices:
[tailscale.md](docs/run-deploy/tailscale.md).

### Talking to the API

Every route except `/health`, `/api/v1/auth/…` and the language reads needs the `session_token`
that signing up or logging in answers:

```sh
curl -H "Authorization: Bearer <session_token>" http://localhost:8080/api/v1/libraries
```

Errors answer `{"error": "<key>"}` with a matching status code. Every route is listed in
`set_up_routes` in [src/routes/api_routes.rs](src/routes/api_routes.rs); the handlers live in
[src/routes/api_caller/](src/routes/api_caller/).

## Highlights

**Library**

- **Four formats:** PDF, EPUB, CBZ and folders of images.
- **Series from file names:** volume, chapter (decimals too) and page numbers are parsed from
  the name, and titles match regardless of case or of how their accents are encoded.
- **Folder watching:** a changed folder is scanned again 10 seconds after its last change, and a
  drive plugged back in is watched and scanned again.
- **Careful, incremental scans:** only changed files are read again, a moved or renamed file
  keeps its row, and nothing is removed from a folder that is missing or unreadable. A scan
  stuck on a network share that stopped answering is given up after 2 minutes.
- **Covers per format,** named by the file's BLAKE3 hash and written atomically; a series or a
  library can show a cover you pick.
- **Streaming with HTTP range requests,** and comic pages one by one. A file is served only when
  its real path, symlinks resolved, lies inside its folder.

**Accounts and security**

- **Accounts:** Argon2id passwords, email verification over SMTP, throttled wrong passwords, and
  admin and member roles.
- **Sessions:** opaque tokens stored only as BLAKE3 hashes — a 15-minute session token and a
  rotating 30-day refresh token.
- **Two-factor login:** TOTP secrets encrypted at rest with XChaCha20-Poly1305, single-use
  recovery codes, replay protection, and a lock on wrong codes that doubles up to 24 hours.
- **Private by default:** listens on `127.0.0.1` only; other devices reach it over HTTPS through
  `tailscale serve`.

**Under the hood**

- **One trait per database operation** (`Extract`, `Insert`, `Update`, `Search`, `Delete`): a
  model names its table and columns, and the trait builds the queries.
- **Transactions as closures:** `setup_new_transaction(|conn| …)` commits on `Ok` and rolls back
  on `Err`.
- **Typed errors:** one `thiserror` enum per domain, each variant mapped to a status code and a
  stable key the app translates.
- **No N+1 queries:** a list loads all its rows in one `IN (…)` query.
- **SQLite behind an r2d2 pool,** with the migrations compiled into the binary.
- **Translations** in English and German, sent with an ETag; mails go out in each user's
  language.
- **Release builds by GitHub Actions** for macOS, Windows and Linux.
- **`cargo test` needs no setup:** in-memory SQLite and temporary files.

More: [architecture](docs/ARCHITECTURE.md) · [development commands](docs/commands.md).

## License

Copyright © 2026 Hendrik Doludda. Licensed under the [MIT License](LICENSE).
