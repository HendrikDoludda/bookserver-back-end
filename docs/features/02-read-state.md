# Read state

State: completed

Each user's place in each book — where they got to, and whether they finished — so any
client can resume, show progress, list what the user is in the middle of, and mark what's done.

## Build
- [x] 2 h · Migration `0010`: `reading_progress` keyed by `(user_id, book_id)` — `position` and `total` (the reader's page, or a flowing EPUB's block, from 0, out of the count the reader counted; both NULL for a book marked read by hand before it was opened), `finished`, `last_read_at` — with foreign keys to `users` and `books`, `ON DELETE CASCADE`; dedicated functions in `db_reading_progress.rs`; never `books.page_number`, which is parsed from the file name [DB]
- [x] 1 h · Save in one upsert — `INSERT … ON CONFLICT (user_id, book_id) DO UPDATE` — keeping the fields a change leaves out; `last_read_at` is set only by a position [BE]
- [x] 2 h · `GET` and `PATCH /books/{id}/progress` with `{"position", "total", "finished"?}`, the user from the session; 400 for an empty body, or a position without its total or outside `0..total` [BE]
- [x] 2 h · `GET /books/{id}` and `GET /series/{id}/books` carry the caller's `progress`; `GET /libraries/{id}/series` carries `finished_count` — one grouped query per list [BE]
- [x] 2 h · App: parse the progress, and save it through `PATCH` once the position rests 2 s and when the reader closes [FE]
- [x] 1.5 h · CBZ/image reader opens at the saved page and reports the last page as finished [FE]
- [x] 1.5 h · PDF reader opens at the saved page and reports the last page as finished [FE]
- [x] 4 h · EPUB reader opens at the saved place — flowing: the block at the top of the screen; fixed layout: the page — and reports the end on screen as finished [FE]
- [x] 1 h · A finished book opens at its start, and its first page turn sends `finished: false`; a saved total that differs from the reader's maps the position proportionally [FE]
- [x] 3 h · A bar along the cover's bottom and a check on finished books and fully read series; the book and series lists reload when the screen above them closes [FE]
- [x] 2 h · Long-press or right-click a book card: Mark as read / Mark as unread [FE]
- [x] 1 h · Check by hand: a CBZ, a PDF and an EPUB read part-way resume where they were left; the cards show bars; reaching the end marks a book read; reopened, a page turn unmarks it; marking by hand; a second account sees none of it
- [x] 2 h · `GET /books/started`: the calling user's started, unfinished books — with a place, not finished — most recently read first, each with its series' name and its progress [BE]
- [x] 3 h · "Continue reading" screen, opened from a button in the home screen's top bar: the started books with their series' names and bars; a tap opens the reader, a long press marks [FE]
- [x] 2 h · `GET /libraries` carries `book_count` and the calling user's `finished_count`, one grouped query each [BE]
- [x] 1 h · Library cards: the bar of finished books, and a check once all are read [FE]
