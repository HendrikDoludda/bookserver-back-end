# Directory scanning

State: in progress

The scanner walks the scannable directories ([35](35-scannable-directories.md)), parses volume /
chapter / page numbers out of file names, and creates the book and series rows plus a cover for
each supported file. It works, but it can report success before it is done, swallows per-file
errors, stops at the first unreadable folder, and never picks up a file that changed.

## Build
- [ ] 6 h · Run every scan as a job through one worker that takes requests from a queue: `POST /scan` answers 202, or 409 `scan_in_progress` while a scan runs or waits; `GET /scan/status` (answers 501 until then, admin only) reports progress, counts and per-file failures, held in memory; queued requests merge. The worker's handle joins `AppState` [BE]
- [ ] 1 h · A scan counts as done only once its last book is written — `scan_folder` drops the `JoinHandle`s today, so the last file can still be in progress when it reports success; the walk runs in one `spawn_blocking` that returns its report, with no per-file tasks to wait for [BE]
- [ ] 2 h · Report per-file failures instead of discarding them — only a task panic is logged; `create_book_entry`'s own errors disappear. A failure carries the directory, the path relative to it and an error key; each book runs inside `catch_unwind`, so a panic costs that file only [BE]
- [ ] 2 h · `create_book_entry` creates a file's series before its cover step and book insert, so a file whose cover step panics (a PDF without PDFium, [36](36-covers.md)) or whose insert fails leaves an empty series behind — extract the cover first, then create the series, link it and insert the book in one transaction, so a failure leaves none of them [BE]
- [ ] 1 h · An image lying directly in a scanned root makes the root itself one image-comic book named after the folder (`../Downloads` became the book "Downloads", every loose image a page) — only a subfolder of images becomes a book, handled once, not once per image [BE]
- [ ] 1 h · A missing or unreadable root, or an unreadable subfolder such as `.Trashes`, aborts the whole scan — report it and carry on; a root that can't be read is reported apart from a missing one [BE]
- [ ] 0.5 h · Walk with `walkdir`, following no symlinks [BE]
- [ ] 4 h · Incremental scanning: a `fingerprint` column — a file's size and nanosecond modification time, an image folder's BLAKE3 over its pages' names, sizes and times — skips an unchanged book without opening it; a changed one is hashed, and a new hash renews its row under the same id. A book without a cover is tried again. Migration 0007 adds the column and an index on `file_hash` — today every file is hashed on every scan, and a stored one is never updated [DB] [BE]
- [ ] 1 h · An image folder's hash covers its pages in page order, its size is their sum and its time the newest page's — today all three come from whichever image `read_dir` lists first [BE]
- [ ] 1 h · Match file extensions case-insensitively (`.PDF` and `.JPG` are skipped today), and accept `.webp` pages, which the cover code already handles [BE]
- [x] 1.5 h · A database error while looking up a series counts as "not found" and creates a duplicate (`get_series_id`) — the lookup becomes a `Database` method that answers the error [BE]
- [ ] 1.5 h · The legacy `get_id_from_table` goes with `ColumnSelector` once nothing calls it — the scanner's check for a stored path is its last caller [BE]
- [x] 2 h · Keep a series' original casing for display — names are stored lowercased because the lowercased name is the duplicate check; match by a `series.match_key`, lowercased in Rust, which a rename keeps. Migration 0005 fills the key and gives existing series their casing back from their books' titles [DB] [BE]
- [x] 1 h · A file joins the series of its `match_key` that holds a book of the same directory, else a new series — today a name matches across the whole database [BE]
- [ ] 3 h · File-name parsing: accept decimal chapters (`c01.5` parses as chapter 1 today) and `.` / `-` separators (`vol.3` and `v-3` don't match) — `chapter_number` becomes a decimal, and the JSON sends a whole chapter as an integer. [Uploads](05-uploads-and-ingestion.md) name files to this same rule set [BE]
- [ ] 2 h · Strip bracketed tags such as `[Group]` or `(Digital)` from parsed titles, so they don't split one series into several [BE]
- [ ] 0.5 h · A folder book's title comes from the folder's whole name — `file_stem` turns "Mr. Robot" into "Mr" [BE]
- [ ] 1 h · A name that leaves no title ("Chapter 12", "v01.cbz") takes its folder's name — today every such book joins one series named "" [BE]
- [ ] 0.5 h · The app reads `chapter_number` as `num?` and shows it in the catalog language's number format — its `int?` pattern rejects a whole series' book list at the first `1.5` [FE]
