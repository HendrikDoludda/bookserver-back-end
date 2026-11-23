# Directory scanning — checks

On the machine with your real library. Restart the server on the new build first — `cargo run` in the repo. `PASTE_USERNAME` and `PASTE_PASSWORD` are your admin account's; `PASTE_SESSION_TOKEN` is the `session_token` from the login answer (good for 15 minutes).

Log in: `curl -X POST http://localhost:8080/api/v1/auth/login -H 'Content-Type: application/json' -d '{"username_or_email":"PASTE_USERNAME","password":"PASTE_PASSWORD","device_name":"terminal","platform":"macos"}' -w '\n'`

- [ ] Start a full scan — it answers `202`: `curl -X POST http://localhost:8080/api/v1/scan -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'`
- [ ] Once it is done, `last` shows `updated` above 0 — the books that took a new title — and no failures: `curl http://localhost:8080/api/v1/scan/status -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'`
- [ ] In the app, a series that was split into one series per volume (`Name v01 Subtitle` files) is one series now, its books shown as Volume 1, Volume 2, …
- [ ] A book named `… Book 3` shows as Volume 3 in its series
- [ ] A name ending in a year — this Mac's `Elf_Receiver_Radio-Craft_August_1936` — keeps the year in its title and has no volume
- [ ] Run both commands again: the second scan's `last` shows `updated` 0 — nothing moves twice
