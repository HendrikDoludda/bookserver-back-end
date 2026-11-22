# Checks — 35 Scannable directories

Run the commands in the repository folder, where `data/databases/` is. `PASTE_SESSION_TOKEN` is the
`session_token` in the answer of [`login`](run-deploy/local.md#commands) as the admin.

- [ ] Stop the server — this step's build changes the database at its start.
- [ ] Back up the database: `sqlite3 data/databases/app_data.sqlite "VACUUM INTO 'data/databases/backup-before-0006.sqlite'"` — prints nothing.
- [ ] Start the server with [`run`](run-deploy/local.md#commands) — its log shows `Applied migration 0006 directories`.
- [ ] Add your book folder with a library of its own: `curl -X POST http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"path":"/Users/hendrikdoludda/Documents/BookserverTest","create_library":true,"library_type":"books"}' -w '\n'` — answers `{"id":1,"path":"/Users/hendrikdoludda/Documents/BookserverTest","library_id":…}`.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT COUNT(*) FROM books WHERE directory_id IS NULL"` prints `0`: all 19 books joined the folder.
- [ ] In the app, the library list shows **BookserverTest** with your series, and a book opens in the reading mode you chose for its series before.
- [ ] The add command again answers `{"error":"directory_taken"}`.
- [ ] `curl -X POST http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"path":"/Users/hendrikdoludda/bookserver_backend","create_library":false}' -w '\n'` answers `{"error":"directory_overlaps"}` — that folder holds the server's database and covers.
- [ ] `mkdir /tmp/bookserver-check`, then add it: `curl -X POST http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"path":"/tmp/bookserver-check","create_library":false}' -w '\n'` — its path comes back as `/private/tmp/bookserver-check`, where `/tmp` really leads, and its `library_id` as `null`.
- [ ] Remove it, with `PASTE_ID` the `id` in that answer: `curl -X DELETE http://localhost:8080/api/v1/directories/PASTE_ID -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '%{http_code}\n'` prints `204`; then `rmdir /tmp/bookserver-check`.
- [ ] `curl http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` lists only the BookserverTest folder.
