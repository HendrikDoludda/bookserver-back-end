# Commands

| Command | What it does | Destructive |
|---|---|---|
| `cargo check` | Type-check without building | no |
| `cargo build` | Build the server | no |
| `cargo test` | Run the automated tests | no |
| `cargo clippy` | Lint | no |
| `cargo fmt` | Format the code | no |
| `sqlite3 data/databases/app_data.sqlite "VACUUM INTO 'data/databases/backup-NAME.sqlite'"` | Back up the database into a new file, with a `NAME` of your choice — a consistent copy even while the server runs; the file must not exist yet | no |
| `sqlite3 data/databases/app_data.sqlite "BEGIN; DELETE FROM library_elements; DELETE FROM books; DELETE FROM series; DELETE FROM library; COMMIT;"` | Empty the catalog — every library, series and book row — and keep accounts, sessions and settings; stop the server first. The next scan fills it again; cover files stay in `cover_images/` | yes |
| `rm -f data/databases/app_data.sqlite*` | Delete the database and its WAL files — every account, session and catalog row; the next start recreates it, empty, from the migrations | yes |
| `sqlite3 data/databases/app_data.sqlite "DELETE FROM login_challenges WHERE user_id = (SELECT id FROM users WHERE username = 'NAME'); DELETE FROM recovery_codes WHERE user_id = (SELECT id FROM users WHERE username = 'NAME'); DELETE FROM totp WHERE user_id = (SELECT id FROM users WHERE username = 'NAME');"` | Turn 2FA off by hand for the account `NAME` that lost both its phone and its recovery codes: deletes its TOTP setup, its recovery codes and its pending logins | yes |
