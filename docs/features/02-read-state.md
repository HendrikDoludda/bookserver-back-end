# Read state

State: in progress

Each user's place in each book — where they got to, and whether they finished — so any
client can resume, show progress, list what the user is in the middle of, and mark what's done.

## Build
- [x] 2 h · Migration `0010`: `reading_progress` keyed by `(user_id, book_id)` — `position` and `total` (the reader's page, or a flowing EPUB's block, from 0, out of the count the reader counted; both NULL for a book marked read by hand before it was opened), `finished`, `last_read_at` — with foreign keys to `users` and `books`, `ON DELETE CASCADE`; dedicated functions in `db_reading_progress.rs`; never `books.page_number`, which is parsed from the file name [DB]
- [x] 1 h · Save in one upsert — `INSERT … ON CONFLICT (user_id, book_id) DO UPDATE` — keeping the fields a change leaves out; `last_read_at` is set only by a position [BE]
- [x] 2 h · `GET` and `PATCH /books/{id}/progress` with `{"position", "total", "finished"?}`, the user from the session; 400 for an empty body, or a position without its total or outside `0..total` [BE]
- [ ] 2 h · `GET /books/{id}` and `GET /series/{id}/books` carry the caller's `progress`; `GET /libraries/{id}/series` carries `finished_count` — one grouped query per list [BE]
- [ ] 2 h · App: parse the progress, and save it through `PATCH` once the position rests 2 s and when the reader closes [FE]
- [ ] 3 h · PDF and CBZ/image readers open at the saved page and report the last page as finished [FE]
- [ ] 4 h · EPUB reader opens at the saved place — flowing: the block at the top of the screen; fixed layout: the page — and reports the end on screen as finished [FE]
- [ ] 1 h · A finished book opens at its start, and its first page turn sends `finished: false`; a saved total that differs from the reader's maps the position proportionally [FE]
- [ ] 3 h · A bar along the cover's bottom and a check on finished books and fully read series; the book and series lists reload when the screen above them closes [FE]
- [ ] 2 h · Long-press or right-click a book card: Mark as read / Mark as unread [FE]
- [ ] 1 h · Check by hand: a CBZ, a PDF and an EPUB read part-way resume where they were left; the cards show bars; reaching the end marks a book read; reopened, a page turn unmarks it; marking by hand; a second account sees none of it
- [ ] 2 h · "Continue reading": the calling user's started, unfinished books, most recently read first, from a collection endpoint of its own [BE]
- [ ] 3 h · "Continue reading" surface [FE]
- [ ] 2 h · Unread-count badges on libraries, one grouped query per list [BE]
- [ ] 1 h · Library unread-count badges [FE]
