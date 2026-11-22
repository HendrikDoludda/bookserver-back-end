# Commands

| Command | What it does | Destructive |
|---|---|---|
| `cargo check` | Type-check without building | no |
| `cargo build` | Build the server | no |
| `cargo test` | Run the automated tests | no |
| `cargo clippy` | Lint | no |
| `cargo fmt` | Format the code | no |
| `sqlite3 data/databases/app_data.sqlite "VACUUM INTO 'data/databases/backup-NAME.sqlite'"` | Back up the database into a new file, with a `NAME` of your choice — a consistent copy even while the server runs; the file must not exist yet | no |
| `sqlite3 data/databases/app_data.sqlite "PRAGMA foreign_keys = ON; BEGIN; DELETE FROM library_elements; DELETE FROM books; DELETE FROM series; COMMIT;"` | Empty the catalog — every series and book row, with the reading modes chosen for those series — and keep accounts, sessions, settings, the added folders and the libraries; stop the server first. The next scan fills each folder's library again; libraries filled by hand stay empty. The `PRAGMA` makes the deletes clear what depends on them, as the server's do; cover files stay in `cover_images/` | yes |
| `rm -f data/databases/app_data.sqlite*` | Delete the database and its WAL files — every account, session and catalog row; the next start recreates it, empty, from the migrations | yes |
| `sqlite3 data/databases/app_data.sqlite "DELETE FROM login_challenges WHERE user_id = (SELECT id FROM users WHERE username = 'NAME'); DELETE FROM recovery_codes WHERE user_id = (SELECT id FROM users WHERE username = 'NAME'); DELETE FROM totp WHERE user_id = (SELECT id FROM users WHERE username = 'NAME');"` | Turn 2FA off by hand for the account `NAME` that lost both its phone and its recovery codes: deletes its TOTP setup, its recovery codes and its pending logins | yes |
