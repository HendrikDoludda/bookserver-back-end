# Checks — 35 Scannable directories

Run the commands in the repository folder, where `data/databases/` is. `PASTE_SESSION_TOKEN` is the
`session_token` in the answer of [`login`](run-deploy/local.md#commands) as the admin.

- [ ] Stop the server and start it again with [`run`](run-deploy/local.md#commands) — the command no longer sets `BOOK_DIRS`, which the server now ignores.
- [ ] Scan: `curl -X POST http://localhost:8080/api/v1/scan -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` answers `Directory scan complete`.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT d.path, COUNT(b.id) FROM directories d LEFT JOIN books b ON b.directory_id = d.id GROUP BY d.id"` prints `/Users/hendrikdoludda/Documents/otherbookservertest|5` and `/Users/hendrikdoludda/Documents/BookserverTest|19` — the 5 PDFs of `otherbookservertest` are new, and nothing in BookserverTest was added twice.
- [ ] In the app, the library **otherbookservertest** shows those 5 PDFs' series.
- [ ] Put a copy of one into your other folder: `cp /Users/hendrikdoludda/Documents/otherbookservertest/laravel-queues-in-action-dark.pdf /Users/hendrikdoludda/Documents/BookserverTest/`, then scan again with the scan command. Keep the copy: deleting it would leave its row until step 19.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT b.directory_id, b.series, e.library_id FROM books b JOIN library_elements e ON e.series_id = b.series WHERE b.title = 'laravel queues in action dark' ORDER BY b.directory_id"` prints two lines, `1|…|8` and `2|…|9`, with a different series id in the middle of each: one series per folder, each in its folder's library.
- [ ] Delete the library `/users/hendrikdoludda/documents/bookservertest` that scans of `BOOK_DIRS` made — id 5 in your database; its series stay in BookserverTest: `curl -X DELETE http://localhost:8080/api/v1/libraries/5 -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '%{http_code}\n'` prints `204`.
- [ ] Delete the library `../downloads` that a scan of the old `BOOK_DIRS` default made — id 10: `curl -X DELETE http://localhost:8080/api/v1/libraries/10 -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '%{http_code}\n'` prints `204`. Its 7 books from `~/Downloads` stay in the database, in no library, until step 19 removes books in no folder.
- [ ] Scan again; `curl http://localhost:8080/api/v1/libraries -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` lists neither of the two — a scan makes no library of its own any more.
