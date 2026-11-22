# Checks — 34 Directory scanning

Run the commands in the repository folder, where `data/databases/` is. `PASTE_SESSION_TOKEN` is the
`session_token` in the answer of [`login`](run-deploy/local.md#commands) as the admin. The scan
command is [`scan`](run-deploy/local.md#commands).

- [ ] Stop the server and start it again with [`run`](run-deploy/local.md#commands).
- [ ] Scan: it answers `{"added":0,"unchanged":25,"failures":[]}` — the 25 books of your two folders are found stored, and none is read again.
- [ ] Make a throwaway folder to scan: `mkdir -p "/tmp/bookserver-scan-check/Mr. Robot" /tmp/bookserver-scan-check/.hidden /tmp/bookserver-scan-check/Locked && cp "/Users/hendrikdoludda/Documents/BookserverTest/Picture Book C1/a-detailed-fern-leaf-in-the-dark.jpg" "/tmp/bookserver-scan-check/Mr. Robot/1.jpg" && cp /Users/hendrikdoludda/Documents/otherbookservertest/laravel-queues-in-action-light.pdf "/tmp/bookserver-scan-check/Shout v01.PDF" && cp "/tmp/bookserver-scan-check/Shout v01.PDF" "/tmp/bookserver-scan-check/._Shout v01.pdf" && cp "/tmp/bookserver-scan-check/Shout v01.PDF" "/tmp/bookserver-scan-check/.hidden/Secret v01.pdf" && cp "/tmp/bookserver-scan-check/Shout v01.PDF" /tmp/bookserver-scan-check/Locked/ && chmod 000 /tmp/bookserver-scan-check/Locked` — prints nothing.
- [ ] Add it without a library: `curl -X POST http://localhost:8080/api/v1/directories -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"path":"/tmp/bookserver-scan-check","create_library":false}' -w '\n'` — answers its `id`, `PASTE_ID` below.
- [ ] Scan: it answers `{"added":2,"unchanged":25,"failures":[{"directory_id":PASTE_ID,"relative_path":"Locked","error_key":"folder_unreadable"}]}` — the upper-case `Shout v01.PDF` and the folder `Mr. Robot` are added, the `._` copy and the hidden folder are skipped, and the locked folder is reported while the rest is scanned.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT title, format FROM books WHERE directory_id = PASTE_ID ORDER BY title"` prints `Mr. Robot|image_comic` and `Shout|pdf` — the folder keeps its whole name.
- [ ] Move the folder away — `mv /tmp/bookserver-scan-check /tmp/bookserver-scan-check-away` — and scan: it answers `added` 0, `unchanged` 25 and the one failure `{"directory_id":PASTE_ID,"relative_path":"","error_key":"directory_missing"}`. Move it back: `mv /tmp/bookserver-scan-check-away /tmp/bookserver-scan-check`.
- [ ] Remove it: `curl -X DELETE http://localhost:8080/api/v1/directories/PASTE_ID -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '%{http_code}\n'` prints `204` and takes its two books out of the catalog; then `chmod 755 /tmp/bookserver-scan-check/Locked && rm -r /tmp/bookserver-scan-check` deletes the throwaway folder and its copies.
- [ ] In the app, your libraries and books show as before.
