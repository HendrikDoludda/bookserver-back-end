# Checks — 34 Directory scanning

Run the commands in the repository folder, where `data/databases/` is.

- [ ] Stop the server — this step's build changes the database at its start.
- [ ] Back up the database: `sqlite3 data/databases/app_data.sqlite "VACUUM INTO 'data/databases/backup-before-0005.sqlite'"` — prints nothing.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT COUNT(*) FROM series"` — note the number.
- [ ] Start the server with [`run`](run-deploy/local.md#commands) — its log shows `Applied migration 0005 series_match_key`.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT name, match_key FROM series ORDER BY id LIMIT 5"` prints each name with the capitals of its books' file names, and its key in lowercase, such as `Berserk|berserk`.
- [ ] In the app, the series grid and Manage → Series show the names with their capitals.
- [ ] Scan with [`scan`](run-deploy/local.md#commands) and the admin's `session_token` from [`login`](run-deploy/local.md#commands); the count from above is unchanged: no series came twice.
- [ ] Only if you have a volume that isn't in the catalog yet: rename its series in Manage → Series (say, add " Deluxe"), copy the file into your book folder and scan — it joins the renamed series instead of starting a new one. Rename it back afterwards.
