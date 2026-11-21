# Checks — 36 Covers

Run the commands in the repository folder, where `data/databases/` is.

- [ ] Stop the server — this step's build changes the database at its start.
- [ ] Back up the database: `sqlite3 data/databases/app_data.sqlite "VACUUM INTO 'data/databases/backup-before-0004.sqlite'"` — prints nothing.
- [ ] Start the server with [`run`](run-deploy/local.md#commands) — its log shows `Applied migration 0004 cover_file_names`. When the server finds no PDFium library, the log also says so once: `The PDFium library can't be loaded, so PDFs get no cover`.
- [ ] `sqlite3 data/databases/app_data.sqlite "SELECT COUNT(*) FROM books WHERE cover_image LIKE './cover_images%'"` prints `0`, and `sqlite3 data/databases/app_data.sqlite "SELECT cover_image FROM books WHERE cover_image IS NOT NULL LIMIT 3"` prints bare file names such as `0390c5….jpg`.
- [ ] In the app, your libraries, series and books show their covers as before.
- [ ] Scan with [`scan`](run-deploy/local.md#commands) and the admin's `session_token` from [`login`](run-deploy/local.md#commands) — the covers still show, and `ls -d cover_images*` lists only the folder `cover_images`, no `cover_images.jpg` beside it.
- [ ] Only if the start log warned about PDFium and you have a PDF that isn't in the catalog yet: copy it into your book folder and scan — it now shows in the app, without a cover. Before this step it stayed out.
