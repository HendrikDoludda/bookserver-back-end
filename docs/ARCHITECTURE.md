# Architecture

## Where things live
- `src/main.rs` — entry point: tracing, database, `start_server`.
- `src/lib.rs` — module declarations; re-exports `data_models::models` and `database_related_scripts::db`, so `crate::models::…` and `crate::db::…` resolve.
- `src/config.rs` — env-var readers, the data folder (`data_folder`) and the generated `secrets.toml` in it.
- `src/app_state.rs` — `AppState`, the router state (database, mailer, setup code, TOTP cipher, covers, scan jobs, folder watch); handlers take their part through `FromRef` (`State<Arc<Database>>`, `State<Mailer>`, `State<SetupCode>`, `State<TotpCipher>`, `State<Covers>`, `State<ScanJobs>`, `State<FolderWatch>`).
- `src/routes/api_routes.rs` — the route table (`set_up_routes`) and `start_server`.
- `src/routes/api_caller/` — the handlers, one file per area (`authentication.rs`, `account.rs`, `settings.rs`, `totp.rs`, `library.rs`, `directories.rs`, `scans.rs`, `users.rs`, `locales.rs`); `extractors.rs` holds the request extractors — the session ones (`AuthSession`, `AdminSession`, `AuthTOTPLoginSession`, `AuthRefreshSession`) and `JsonBody` / `PathParams` / `QueryParams`, and `helpers.rs` the shared response builders: `ping_server` (the liveness check), `not_implemented` (every route without a handler yet) and the router's fallbacks `not_found` / `method_not_allowed`.
- `src/routes/auth.rs` — account, password and session building blocks, including the checks a new username, email or password passes; `login.rs` — login, refresh and logout; `registration.rs` — sign-up and email verification; `password_reset.rs` — the reset flow; `account_management.rs` — a user's own account: reading it, and the changes they make to it; `email_codes.rs` — the emailed codes of sign-up, reset and the email change: making them, mailing them within the limits, checking them; `totp_management.rs` — TOTP two-factor and recovery codes; `totp_cipher.rs` — `TotpCipher`, which encrypts TOTP secrets at rest; `email_helper.rs` — `Mailer`, the one sender every email goes through; `user_management.rs` — an admin's view of the accounts: listing them, changing a role, marking an email verified; `user_settings.rs` — a user's own settings: reading them, and replacing them once each value passes its check; `translations.rs` — the catalog as the apps read it: the languages on offer, and a language's texts with the English fallback; `library_management.rs` — what admins change about libraries and series: creating and editing libraries, which series they hold, editing and deleting series, and the covers both show; `directory_management.rs` — which folders the scanner reads: adding one, with a library of its own when asked and the stored books it adopts, listing them, and removing one with its books; `covers.rs` — which cover file a book, a series or a library shows; `served_files.rs` — the file on disk a book or a cover request reads, once it is known to lie inside the book's directory or the cover folder; `reading_modes.rs` — which mode a user reads a book in, and storing their choice for its series.
- `src/database_related_scripts/` — `db.rs` (`Database`: the r2d2 pool and every DB operation), one file per operation trait, `db_email_codes.rs` / `db_login_challenges.rs` / `db_login_throttle.rs` (one table each), `db_users.rs` (the user queries the traits can't express, and how a `Role` is stored), `db_reading_modes.rs` (the `reading_modes` table, and how a `ReadingMode` and a book's `ReadingDirection` are stored), `db_translations.rs` (the translation catalog's three tables, and how an `Audience` is stored), `migrations.rs`, `translation_seed.rs` (checks the shipped translations and syncs them into the catalog at every start).
- `src/data_models/` — `models.rs` (domain structs, `*DatabaseColumns` enums, `WithId<T>`) and the request/response bodies.
- `src/error_types.rs` — the typed error enums, one per domain.
- `src/scanner/` — `scan_jobs.rs` (`ScanJobs`: the one worker that runs every scan in the background, one at a time, and the status it publishes), `folder_watch.rs` (`FolderWatch`: a system watcher of its own for each added directory, the task that queues a scan of each folder that changed once it rested, and the one that looks at every directory each minute and watches one that came back again), `heartbeat.rs` (`Heartbeat`: a running scan's signs of life, and the worker's word that it gave up on the scan), `folder_scanner.rs` (the scan: walks each added directory, or folder inside one, once, stores each book file and folder of pages in a transaction of its own, parses file names, and reports what it couldn't read or store), `cover_image_retriever.rs` (`Covers` — the cover folder, made, resolved and given the fallback cover at start, and the PDFium library, bound once at start from the binary's folder, else the working directory, else the system's library path — and one cover per format, named by the file's hash), `comic_info.rs` (the reading direction a CBZ's or an image folder's `ComicInfo.xml` states), `epub_package.rs` (the cover an EPUB's package document names, read without the rest of the book), `metadata_fetcher.rs` (placeholder).
- `src/stream_reader.rs` — `serve_file`: a file on disk, with HTTP range support.
- `src/book_pages.rs` — the pages of an image-folder or CBZ book, in natural order and numbered from 1: the one listing behind the page routes and the cover picker; and each page's size.
- `src/test_support.rs` — shared test fixtures, compiled only for tests.
- `migrations/` — numbered SQL files, embedded at compile time.
- `assets/` — `example-cover.png`, the fallback cover, embedded at compile time.
- `translations/` — the shipped translations, embedded at compile time: `<language>/language.json`, and one JSON file per namespace under `frontend/` (sent to the apps) or `backend/` (rendered by the server).

## API
- API-first: every change to data is an HTTP endpoint; the frontend and the admin view are both clients of that same API, no endpoint is reachable from only one of them, and business logic stays in the backend.
- Every route lives under `/api/v1/`, except `/health`.
- Routes are resource paths — plural nouns and `{id}` segments, never verbs: `GET /libraries/{id}/series`, not `/get_series_from_library/{id}`.
- A handler reads its body with `JsonBody<T>`, its path with `PathParams<T>` and its query string with `QueryParams<T>`, never with axum's `Json`, `Path` or `Query`; at most one body extractor, and it comes last.
- `user_id` always comes from the session (`AuthSession`), never from the request body or path.
- Every route needs a session except `/health`, the public `/api/v1/auth/…` routes and the catalog's reads (`GET /api/v1/locales`, `GET /api/v1/locales/{code}/translations`); admin actions need `AdminSession`. The extractor is a handler argument, so the signature shows who may call it — `not_implemented` placeholders included.
- Session, refresh and challenge tokens travel as `Authorization: Bearer <token>`; every failed token check answers the same 401, and a response that carries tokens sends `Cache-Control: no-store`.
- A JSON answer that apps keep, like the translation catalog, carries an `ETag` — the BLAKE3 hash of its body — and `Cache-Control: no-cache`; an `If-None-Match` naming it answers 304 without the body.
- Tokens are stored only as `hash_token` (BLAKE3) hashes and looked up by hash; passwords and emailed codes, which people choose or type, are Argon2-hashed; recovery codes, 80 random bits, are BLAKE3-hashed like tokens.
- TOTP secrets are stored encrypted with `TotpCipher` (the user id as associated data), not hashed — checking a code needs the secret itself.
- An account endpoint re-checks the password with `check_current_password` (`login.rs`): login's throttle, and 400 `incorrect_password` for a wrong one — never login's 401, which the app reads as an ended session.
- A permission check is an Axum extractor composed on `AuthSession` (as `AdminSession` is), not an `if` inside handlers.
- Auth answers never reveal whether an account exists: an unknown user and a wrong password get the same response, and so do password-reset and verification-resend requests for an unknown address. The exceptions answer 409 for a taken one: sign-up for an email or username, the username change for a username, and the email change for an email — sign-up reveals it anyway.
- Every stored email address and username has passed `check_email` / `check_username` (`auth.rs`).
- Every stored language setting has passed `check_locale` (`user_settings.rs`) — a language the server offers — in the transaction that stores it.
- Never return or `Debug`-log a struct that carries secrets — `UserMetadata`, `Sessions`, `TOTP` and `RecoveryCodes` derive `Serialize` / `Debug` over the password hash, the token hashes or the TOTP secret; build a response struct. Request bodies that carry a password or a code, `EmailConfig` (the SMTP password) and `EmailInformation` (a body carries codes) don't derive `Debug` at all.
- List endpoints return `WithId<T>`: the row id plus the metadata, flattened in JSON.
- A scan runs only through `ScanJobs`, in the background and one at a time: an admin's request answers 202 at once, or 409 `scan_in_progress` while one runs or waits; the server's own — a directory just added, a folder the watcher saw change — joins the scan that waits.
- In a PATCH body a field left out stays as it is, and `null` clears a field that can be empty — such a field is read with `present` (`books_model.rs`).

## Errors
- One `thiserror` enum per domain, in `error_types.rs`.
- Every error answer is JSON `{"error": "<key>"}` — a stable key the app translates; a 500 answers `internal_error` and logs the detail server-side.
- An error enum that reaches a handler implements `IntoResponse` through a `status_and_key` match with one arm per variant and no `_` arm, so a new variant can't silently become a 500.
- A 429 sends `Retry-After`: the seconds left on the lock, rounded up, carried by `TooManyAttempts`.
- A route whose handler isn't built answers 501 `not_implemented`; a path no route matches answers 404 `not_found`, a method the route lacks 405 `method_not_allowed`.
- A handler returns `Result<_, AppErrors>` and uses `?`; an error type without `IntoResponse` (such as `anyhow`) is logged and answered with `internal_error_response()`.
- A scan answers what it couldn't read in its report (`ScanProblem` keys), never as an error; a crate's panic on one book is caught (`catch_unwind`) and reported as that book's `panicked`.
- A setting the server can't use stops it at start with an `ApplicationSetUpErrors` naming the setting — never a panic.

## Email
- Mail is sent inline only at sign-up, whose caller just created the account and waits for its code; every other mail (reset and resent verification codes, upload notices) goes out from a background task after the commit, so an answer never shows whether a mail went out.
- Mail goes to the address stored on the account — for an email change, to the new address stored with its code — never straight to one from the request.
- A mail's subject and body come from the catalog's `email` namespace, in the recipient's language (`db_translations::recipient_locale`: their setting while the server offers it, else the default), with its `{placeholders}` filled by `interpolate`.

## Logging
- Log with `tracing`; the `log` crate is not a dependency.
- Never log a password, a token (plain or hashed), a TOTP secret, a verification, reset or recovery code, the signing key, or an email body — bodies carry codes.
- The one exception: the first-sign-up setup code, logged at start while no account exists.
- A rusqlite error is logged once, where it becomes a `DatabaseError` — `map_err(logged(action, what, variant))` (`db.rs`); callers don't log it again.

## Files
- Everything the server writes — `app_data.sqlite`, `covers/`, `secrets.toml` — lies in the data folder (`config::data_folder`: `DATA_DIR`, else the OS's local data folder for `com.hendrikdoludda.bookserver`), never in the working directory or the repo.
- Files are served by row id, never by a path the client supplies.
- A cover row names a bare file in the cover folder that `AppState` holds (`Covers::path_of`), never a path.
- A cover file is written under a temporary name and renamed into place (`write_cover`), so it is whole or absent.
- A stored path is served only once its real location — every symlink resolved — lies inside its book's directory, or a cover's inside the cover folder (`served_files.rs`); the scan follows no symlink and reads no hidden entry — a dot name or `@eaDir` — and a symlink in an image folder is never a page.
- Files are served through `serve_file` (`stream_reader.rs`): `tower_http`'s `ServeFile` — Range, 206, `If-Range` and `HEAD` included — once the path is checked to be a file, so a missing file or a folder answers the JSON `not_found`. Never hand-rolled range handling.
- A page inside an archive is read whole, in `spawn_blocking` and up to `MAX_PAGE_BYTES`, and sent with its media type; `serve_file` is for files on disk.
- An image's width and height come from its header through `imagesize`, never from decoding it; inside an archive only the entry's start is read (`book_pages::page_sizes`).
- A filesystem path built from request input is sanitised, canonicalised and checked to lie inside a scanned directory before it is read or written — except a directory being added, which becomes one: canonicalised, it must be an existing folder that lies neither inside nor around another directory, the cover folder or the database's folder.
- A scan runs on a thread of its own, which `ScanJobs` leaves behind once it shows no sign of life for 2 minutes; it writes only through `transaction` (`folder_scanner.rs`), which checks inside the write lock that the scan wasn't given up on.
- A call into an added directory or its watcher from async code runs through `within` (`folder_watch.rs`), under a time limit: on a share that stopped answering it never returns.
- Blocking work — PDFium, `zip`, `image`, `imagesize`, hashing — runs in `spawn_blocking`, never directly in an async handler.

## Database
- One trait per operation — `Extract`, `Insert`, `Update`, `Search`, `Delete`, `FromRow` — implemented per model; `Database` methods wrap them with a pooled connection.
- Every `Database` method has a `*_with_connection` twin taking `&Connection`, so several operations can share one transaction (`setup_new_transaction`).
- A read-only lookup runs through `Database::read` — no transaction, so it never waits for the write lock.
- A transaction holds only SQL: it starts `IMMEDIATE`, an async handler runs it through `Database::transact` (on Tokio's blocking pool), Argon2 runs in `spawn_blocking` outside it, and mail goes out after the commit.
- Inside a transaction closure, use the `conn` it was given — a pooled `db.*` call opens a second connection that waits on the first one's write lock.
- Use the generic functions, also for a write that depends on what is stored: inside an `IMMEDIATE` transaction no other writer can step between a read, a compare in Rust and the write. Hand-written SQL only for what they can't express.
- A condition other than equality is a `Comparison` in the generic functions (`Delete::delete_where`), not hand-written SQL.
- A try at a secret (an emailed, TOTP or recovery code) is counted and committed before the secret is checked — counted in the same transaction as a failed check, it is rolled back with it; the final transaction re-checks what it uses up and changes exactly one row.
- Every connection is set up by `db::configure` — SQLite applies `foreign_keys` per connection, not per file.
- Values are always bound parameters; table and column names are interpolated only from trusted enums and consts, never from request input.
- Each table names its key column (`KEY` on `Extract` and `Update`, `DatabaseTypes::get_key_column`); nothing assumes it is `id`.
- Tables without a single key column (the `library_elements` junction, `email_codes`, `reading_modes`) don't implement `Extract`; dedicated functions query them — `Database` methods for the junction, `db_email_codes.rs` for the codes, `db_reading_modes.rs` for the modes.
- The translation catalog's tables (`locales`, `translation_keys`, `translations`) are queried only through `db_translations.rs`.
- A list reads its rows in one query — `extract_all` for a whole table, or the ids first, in the list's order, and then one `IN (…)` query (`extract_batch`), which keeps that order — never one query per row.
- An API answer never carries a server file path or a file hash: a book goes out as `BookSummary`, and its file and cover are reached by its id. The exceptions are an admin's answer about a scanned directory, which carries that directory's own path, and a scan's status, whose scope names the folders it reads and whose failures carry a path inside their directory.
- Every reference to another table is a declared `FOREIGN KEY` — `PRAGMA foreign_keys = ON` enforces only declared ones.
- Times are stored as `INTEGER` unix seconds and read with `read_time` (must have a value — NULL is an error naming the column) or `read_optional_time` (NULL is `None`).
- A column is nullable exactly when its Rust field is an `Option`.
- The scanner finds a series by `match_key` — the title it was first found under, lowercased in Rust — among the series holding a book of the same directory; never by `name`, which an admin edits.
- The scan reads a book's file outside any transaction, then stores its series, library link and row in one `IMMEDIATE` transaction of their own — never one per scan, which would hold the write lock for its whole length.
- No file-system call runs inside a transaction, or while a pooled connection is held: on a share that stopped answering it would keep that connection for good.
- A scan opens a stored book's file only when its `fingerprint` changed — a file's size and nanosecond time, an image folder's digest of its pages' — or when it still lacks a cover; a changed file renews its row under the same id and series.
- A scan reads folders — an added directory's root, or a folder inside one — each with everything it holds; a folder that is gone, lies behind a symlink or is spelled in another case than on disk is left out.
- A scan reconciles after its inserts: a new file first takes over the row with its hash that the scan lost, then the rows whose files no walk met are deleted — never in a directory whose root is missing or unreadable, or holds no book when read whole, nor under a folder the walk couldn't read — and the series they leave without books.
- Back up SQLite with `VACUUM INTO`, never by copying the file — under WAL a plain copy is not a valid backup.
- Schema changes: a new numbered `.sql` in `migrations/`, registered in `MIGRATIONS`; an applied migration is never edited.
- A migration rebuilds a table — create, copy with the ids and `sqlite_sequence`'s counter, drop, rename — only while no other table references it: migrations run with foreign keys on, so dropping a referenced table fires its children's `ON DELETE`. Otherwise it adds columns with `ALTER TABLE … ADD COLUMN`.

## Tests
- A test lives in a `#[cfg(test)] mod tests` at the bottom of the file it tests; shared fixtures live in `src/test_support.rs`.
- A database test runs on `memory_connection()`: in-memory, configured, migrated and synced with the shipped translations, fresh for every test. A test of the schema or of the sync itself uses `migrated_connection()`, whose tables are empty.
- A test that needs the pool or the router uses `TempDatabase` — never a plain `:memory:` path, which gives every pooled connection its own empty database.
- A test that needs files on disk writes them into a `TempDir`, which deletes its folder when dropped.
- Router tests send requests with `tower::ServiceExt::oneshot` and read bodies with `axum::body::to_bytes`.
- Fixtures insert rows with plain SQL, so they don't depend on the model code under test.
- Logic that depends on the time takes `now` as a parameter, so tests never sleep; a task that waits on tokio's clock is tested under `#[tokio::test(start_paused = true)]`, and a system event is awaited with a timeout. The scan watchdog is the exception: its scan runs on a thread of its own, which the paused clock would run ahead of, so it is tested in real time with a limit of milliseconds.
- A test waits for a scan with `ScanJobs::idle`, never by sleeping; one that must hold a scan back uses `ScanJobs::without_worker`.

## Modules
- Core code — auth, users, sessions, settings, email, config, migrations — never imports book-specific types (`BookMetadata`, `BookSeriesMetadata`, `BookFormat`, the filename parser); book code may use core, never the reverse. Today's mixed files (`models.rs`, the per-model trait impls, `api_caller/library.rs`) get split up by [94](features/94-modular-core.md).
- Per-user data references `users.id` through a declared foreign key, in the one SQLite database.
- Format-specific behaviour — filename parsing, cover extraction, reading positions — is chosen by `LibraryType` / `BookFormat` in one place, never spread through core.

## Hosting
- A home server on the LAN, reached remotely only through Tailscale — never public port-forwarding. Hardening that only matters on the open internet lives in [54](features/54-public-exposure-hardening.md).
- HTTPS ends at a TLS proxy in front of the server — `tailscale serve` ([run-deploy](run-deploy/tailscale.md)); the server speaks plain HTTP on `BIND_ADDRESS`, by default `127.0.0.1`, so the LAN gets no plain connection.
