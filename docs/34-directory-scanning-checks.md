# Checks — 34 Directory scanning

Run the commands in the repository folder. `PASTE_SESSION_TOKEN` is the `session_token` in the
answer of [`login`](run-deploy/local.md#commands) as the admin. The commands named below are in
[local.md](run-deploy/local.md#commands).

- [ ] Stop the server and start it again with `run`.
- [ ] `scan-status` answers `{"state":"idle","current":null,"pending":null,"last":null}` — nothing has run since the start.
- [ ] Make a slow folder: `mkdir -p /tmp/bookserver-slow-check && mkfile -n 8g "/tmp/bookserver-slow-check/Big v01.pdf"` — an 8 GB file of zeros that takes no disk space but some seconds to hash.
- [ ] Add it: `curl -X POST http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"path":"/tmp/bookserver-slow-check","create_library":false}' -w '\n'` answers its `id`, `PASTE_ID` below, and its first scan starts by itself.
- [ ] Right after, while the file is hashed — about 15 seconds —, `scan-status` shows `"state":"running"` and a `current` with `"triggers":["directory_added"]`, `"phase":"reading"` and `"found":1`, and `scan` answers `{"error":"scan_in_progress"}`.
- [ ] Once it's done, `scan-status` shows `"state":"idle"` and a `last` with `"scope":{"directories":[PASTE_ID]}`, `"outcome":"completed"` and `"added":1`.
- [ ] Scan BookserverTest alone: `curl -X POST http://localhost:8080/api/v1/directories/2/scan -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` answers with `"pending":{"scope":{"directories":[2]},"triggers":["request"]}`; then `scan-status` shows a `last` with that scope, `"added":0` and `"unchanged":20`.
- [ ] `curl -X POST http://localhost:8080/api/v1/directories/999/scan -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '%{http_code}\n'` prints `{"error":"not_found"}404`.
- [ ] `scan` answers with `"pending":{"scope":"all","triggers":["request"]}`; then `scan-status` shows a `last` with `"scope":"all"`, `"added":0`, `"unchanged":26` and `"failures":[]`.
- [ ] Remove the slow folder: `curl -X DELETE http://localhost:8080/api/v1/directories/PASTE_ID -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '%{http_code}\n'` prints `204`; then `rm -r /tmp/bookserver-slow-check` deletes the folder and its file.
- [ ] In the app, your libraries and books show as before.
