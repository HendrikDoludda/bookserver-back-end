# Checks — 12 Security hardening

Run the commands in the repository folder. `PASTE_SESSION_TOKEN` is the `session_token` in the
answer of [`login`](run-deploy/local.md#commands).

- [ ] Stop the server and start it again with [`run`](run-deploy/local.md#commands).
- [ ] In the app, a PDF, an EPUB, a CBZ and an image-folder book of BookserverTest — `Picture Book C1` is one — and a PDF of otherbookservertest each open and turn their pages, and the library, series and book covers show as before.
- [ ] A book in no added folder is refused: `curl http://localhost:8080/api/v1/books/53/file -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n%{http_code}\n'` prints `{"error":"not_found"}` and `404` — book 53 is `../Downloads/React Native Feature overview English.pdf`, which streamed until now — and the server's log shows `book 53 lies in no directory, so its file is not served`.
