# Directory scanning

State: completed

The scanner walks the scannable directories ([35](35-scannable-directories.md)), parses volume /
chapter / page numbers out of file names, and creates the book and series rows plus a cover for
each supported file, reporting what it couldn't read. Scans run in the background, one at a time,
and read a stored book's file again only when it changed.

## Build
- [x] 6 h · Run every scan as a job through one worker that takes requests from a queue: `POST /scan` answers 202, or 409 `scan_in_progress` while a scan runs or waits; `GET /scan/status` (answers 501 until then, admin only) reports progress, counts and per-file failures, held in memory; queued requests merge. The worker's handle joins `AppState` [BE]
- [x] 1 h · A scan counts as done only once its last book is written — `scan_folder` drops the `JoinHandle`s today, so the last file can still be in progress when it reports success; the walk runs in one `spawn_blocking` that returns its report, with no per-file tasks to wait for [BE]
- [x] 2 h · Report per-file failures instead of discarding them — only a task panic is logged; `create_book_entry`'s own errors disappear. A failure carries the directory, the path relative to it and an error key; each book runs inside `catch_unwind`, so a panic costs that file only [BE]
- [x] 2 h · `create_book_entry` creates a file's series before its cover step and book insert, so a file whose cover step panics (a PDF without PDFium, [36](36-covers.md)) or whose insert fails leaves an empty series behind — extract the cover first, then create the series, link it and insert the book in one transaction, so a failure leaves none of them [BE]
- [x] 1 h · An image lying directly in a scanned root makes the root itself one image-comic book named after the folder (`../Downloads` became the book "Downloads", every loose image a page) — only a subfolder of images becomes a book, handled once, not once per image [BE]
- [x] 1 h · A missing or unreadable root, or an unreadable subfolder such as `.Trashes`, aborts the whole scan — report it and carry on; a root that can't be read is reported apart from a missing one [BE]
- [x] 0.5 h · Walk with `walkdir`, following no symlinks [BE]
- [x] 0.5 h · Skip hidden entries — a dot name or `@eaDir` — so macOS's `._` copies and `.Trashes` on an external drive, or a NAS's thumbnails, never become books or failures [BE]
- [x] 4 h · Incremental scanning: a `fingerprint` column — a file's size and nanosecond modification time, an image folder's BLAKE3 over its pages' names, sizes and times — skips an unchanged book without opening it; a changed one is hashed, and a new hash renews its row under the same id. A book without a cover is tried again. Migration 0007 adds the column and an index on `file_hash` — today a stored file is skipped by its path alone, so a changed one is never updated [DB] [BE]
- [x] 1 h · An image folder's hash covers its pages in page order, its size is their sum and its time the newest page's — today all three come from its page 1 [BE]
- [x] 1 h · Match file extensions case-insensitively (`.PDF` and `.JPG` are skipped today), and accept `.webp` pages, which the cover code already handles [BE]
- [x] 1.5 h · A database error while looking up a series counts as "not found" and creates a duplicate (`get_series_id`) — the lookup becomes a `Database` method that answers the error [BE]
- [x] 1.5 h · The legacy `get_id_from_table` goes with `ColumnSelector` once nothing calls it — the scanner's check for a stored path is its last caller [BE]
- [x] 2 h · Keep a series' original casing for display — names are stored lowercased because the lowercased name is the duplicate check; match by a `series.match_key`, lowercased in Rust, which a rename keeps. Migration 0005 fills the key and gives existing series their casing back from their books' titles [DB] [BE]
- [x] 1 h · A file joins the series of its `match_key` that holds a book of the same directory, else a new series — today a name matches across the whole database [BE]
- [x] 3 h · File-name parsing: accept decimal chapters (`c01.5` parses as chapter 1 today) and `.` / `-` separators (`vol.3` and `v-3` don't match) — `chapter_number` becomes a decimal, and the JSON sends a whole chapter as an integer. [Uploads](05-uploads-and-ingestion.md) name files to this same rule set [BE]
- [x] 2 h · Strip bracketed tags such as `[Group]` or `(Digital)` from parsed titles, so they don't split one series into several [BE]
- [x] 0.5 h · A folder book's title comes from the folder's whole name — `file_stem` turns "Mr. Robot" into "Mr" [BE]
- [x] 1 h · A name that leaves no title ("Chapter 12", "v01.cbz") takes its folder's name — today every such book joins one series named "" [BE]
- [x] 0.5 h · The app reads `chapter_number` as `num?` and shows it in the catalog language's number format — its `int?` pattern rejects a whole series' book list at the first `1.5` [FE]
- [x] 0.5 h · `Book` followed by a number is a volume marker like `v`, `vol` and `volume`: `Harry Potter Book 3` is volume 3 — and `Picture book 2` volume 2 of `Picture` [BE]
- [x] 1.5 h · A name with a volume, chapter or page marker takes only the text before its first marker as its title — `Berserk v01 The Black Swordsman` joins `Berserk`, `One Piece c001 Romance Dawn` joins `One Piece`; with nothing before the marker (`v01 Berserk`) the title stays the name without its markers [BE]
- [x] 1 h · A name without a marker whose last word is a whole number of 1 to 3 digits, after a title, has that number as its volume — `Picture book 2` is volume 2 of `Picture book`; a lone number (`1984`), four digits (`August 1936`) and a decimal (`Version 2.0`) stay in the title [BE]
- [x] 2.5 h · A scan parses a stored book's name again: one that parses differently takes its new title and numbers under the same id and moves to the series its name finds, and the series it leaves is deleted once empty — today a stored book keeps the title, numbers and series of its first scan, so a new parsing rule never reaches the books already stored [BE]
- [x] 1 h · An accented letter written as one character (`ä`) or as a letter and an accent mark (`a` + `◌̈`) finds the same series: the match key is lowercased, then composed (NFC) with the `unicode-normalization` crate — older macOS disks store names the second way, downloads the first, and today the key tells them apart [BE]
- [x] 2 h · Migration 0008 composes every stored match key; a migration may run a Rust step after its SQL, in its transaction, as SQLite can't normalize Unicode — without it a series stored under a decomposed key stops matching its own new books [DB] [BE]
