-- A user's place in a book as its reader counts it — a page, or a flowing EPUB's block, from 0 —
-- out of `total`; both NULL for a book marked read by hand before it was opened.
CREATE TABLE reading_progress (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    position INTEGER,
    total INTEGER,
    finished INTEGER NOT NULL DEFAULT 0 CHECK (finished IN (0, 1)),
    last_read_at INTEGER,
    PRIMARY KEY (user_id, book_id),
    CHECK ((position IS NULL) = (total IS NULL)),
    CHECK (position >= 0 AND position < total)
);

-- Deleting a book looks up its rows by book, and the primary key starts with the user.
CREATE INDEX reading_progress_by_book ON reading_progress (book_id);
