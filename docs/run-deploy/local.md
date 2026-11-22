# Local

## Commands
| Key | Command |
|---|---|
| `run` | `cargo run` |
| `mailpit` | `brew install mailpit && mailpit` |
| `run-mail` | `SMTP_HOST=localhost SMTP_PORT=1025 SMTP_TLS=none SMTP_FROM=server@localhost SMTP_FROM_NAME="Book Server" cargo run` |
| `health` | `curl http://localhost:8080/health` |
| `sign-up` | `curl -X POST http://localhost:8080/api/v1/auth/register -H 'Content-Type: application/json' -d '{"email":"PASTE_EMAIL","username":"PASTE_USERNAME","password":"PASTE_PASSWORD","password_repeat":"PASTE_PASSWORD","device_name":"terminal","platform":"macos","setup_code":"PASTE_SETUP_CODE"}' -w '\n'` |
| `login` | `curl -X POST http://localhost:8080/api/v1/auth/login -H 'Content-Type: application/json' -d '{"username_or_email":"PASTE_USERNAME","password":"PASTE_PASSWORD","device_name":"terminal","platform":"macos"}' -w '\n'` |
| `add-directory` | `curl -X POST http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"path":"PASTE_FOLDER","create_library":true,"library_type":"books"}' -w '\n'` |
| `directories` | `curl http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` |
| `scan` | `curl -X POST http://localhost:8080/api/v1/scan -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` |
| `scan-status` | `curl http://localhost:8080/api/v1/scan/status -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` |

## Run
1. For PDFs: put the PDFium shared library (`libpdfium.dylib`, `libpdfium.so` or `pdfium.dll`, prebuilt at [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)) in the working directory or on the library path — build 7543 or newer, see the `pdfium-render` line in `Cargo.toml`.
2. Start the server — [`run`](#commands). `PORT` defaults to 8080 and `BIND_ADDRESS` to `0.0.0.0` (every interface); for HTTPS, see [tailscale.md](tailscale.md). Email is off unless `SMTP_HOST` is set; then `SMTP_PORT`, `SMTP_FROM` and `SMTP_FROM_NAME` are required, `SMTP_USERNAME` and `SMTP_PASSWORD` are optional but go together, and `SMTP_TLS` is `starttls` (default), `implicit` or `none`. A bad `PORT` or `BIND_ADDRESS`, a `DATA_DIR` that isn't a folder, or an incomplete mail setup, stops the server at start. Everything the server writes — the database `app_data.sqlite`, the `covers/` and `secrets.toml` — lies in its data folder, which the start log names: `DATA_DIR` if set, else on macOS `~/Library/Application Support/com.hendrikdoludda.bookserver/`.
3. To read the server's mail while testing, start Mailpit in a second terminal — [`mailpit`](#commands) — and start the server with [`run-mail`](#commands) instead. Every mail shows up at http://localhost:8025.
4. Check that it answers — [`health`](#commands).
5. Sign up the first account — [`sign-up`](#commands), with `PASTE_SETUP_CODE` from the server's start log. It becomes the admin; its answer's `session_token` is good for 15 minutes, and [`login`](#commands) gives a new one.
6. Add each book folder — [`add-directory`](#commands), with the admin's `session_token` and the folder's full path; its first scan starts by itself. With `create_library` false, its series join no library; [`directories`](#commands) lists the added folders.
7. Follow the scan — [`scan-status`](#commands): `current` shows how far it got, and `last` how the last one ended and what it couldn't read. To scan every folder again — [`scan`](#commands).
