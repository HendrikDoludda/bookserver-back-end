# Downloadable builds — checks

Before the checks, move your dev data into the new data folder, in this order:

1. Stop the running server: Ctrl-C in its terminal. Don't start it again until step 6. A start before the copy creates an empty database there, and step 3 then fails.
2. `mkdir -p "$HOME/Library/Application Support/com.hendrikdoludda.bookserver"`
3. `sqlite3 data/databases/app_data.sqlite "VACUUM INTO '$HOME/Library/Application Support/com.hendrikdoludda.bookserver/app_data.sqlite'"`
4. `cp -R cover_images "$HOME/Library/Application Support/com.hendrikdoludda.bookserver/covers"`
5. `cp -p "$HOME/Library/Application Support/com.placeholder_organization.bookserver_backend/secrets.toml" "$HOME/Library/Application Support/com.hendrikdoludda.bookserver/secrets.toml"`
6. `cargo run` (it rebuilds first).

The old `data/`, `cover_images/` and the old `secrets.toml` stay where they are.

- [ ] The start log shows `bookserver 0.1.0, data folder /Users/hendrikdoludda/Library/Application Support/com.hendrikdoludda.bookserver`, and no `No account exists yet` line.
- [ ] Your account with 2FA still logs in, which shows `secrets.toml` came along:
  1. Log in: `curl -X POST http://localhost:8080/api/v1/auth/login -H 'Content-Type: application/json' -d '{"username_or_email":"PASTE_USERNAME","password":"PASTE_PASSWORD","device_name":"terminal","platform":"macos"}' -w '\n'`. It answers `"status":"totp_required"` with a `challenge_token`.
  2. Send the code: `curl -X POST http://localhost:8080/api/v1/auth/login/totp -H 'Authorization: Bearer PASTE_CHALLENGE_TOKEN' -H 'Content-Type: application/json' -d '{"code":"PASTE_CODE"}' -w '\n'`. `PASTE_CHALLENGE_TOKEN` is the `challenge_token` from the first answer, and `PASTE_CODE` is the 6 digits your authenticator app shows now, not a recovery code.
  3. The answer carries a `session_token`. A new signing key would answer an error here instead.
- [ ] In the app (debug build on this Mac, `http://localhost:8080`), your libraries and their covers show as before.
