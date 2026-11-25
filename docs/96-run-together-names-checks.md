# Run-together file names — checks

Restart the server on the new build first — `cargo run` in the repo. `PASTE_USERNAME` and `PASTE_PASSWORD` are your admin account's; `PASTE_SESSION_TOKEN` is the `session_token` from the login answer (not the `refresh_token`; good for 15 minutes).

Log in: `curl -X POST http://localhost:8080/api/v1/auth/login -H 'Content-Type: application/json' -d '{"username_or_email":"PASTE_USERNAME","password":"PASTE_PASSWORD","device_name":"terminal","platform":"macos"}' -w '\n'`

- [ ] Make a new folder `Hellboy test` inside a folder the server already scans, and copy any CBZ you own into it twice, named `HellboyBPRDVolume10Title.cbz` and `HellboyBPRDVolume11Title.cbz`. The watcher scans it once the folder rests; to scan at once, this answers `202`: `curl -X POST http://localhost:8080/api/v1/scan -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'`
- [ ] In the app, that library shows one series "Hellboy BPRD", its books shown as Volume 10 and Volume 11
