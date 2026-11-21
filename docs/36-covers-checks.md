# Checks — 36 Covers

- [ ] Restart the server with this step's build — [`run`](run-deploy/local.md#commands).
- [ ] Put a real `.epub` that has a cover and isn't in your catalog yet into your book folder — a bought one, or a free one from standardebooks.org. A copy of a book you already have tests nothing: its cover is found in `cover_images/` by the file's hash before the book is opened.
- [ ] Scan with [`scan`](run-deploy/local.md#commands) and the admin's `session_token` from [`login`](run-deploy/local.md#commands) — the server log shows no `Failed cover creation` line for that book.
- [ ] In the app, the new book shows its actual front cover.
