-- A path is stored canonical — absolute, every symlink resolved — so no folder can be added twice
-- under two spellings.
CREATE TABLE directories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    -- A deleted library leaves its folder scanning into no library.
    library_id INTEGER REFERENCES library(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- NULL for a book scanned before folders were stored, until adding its folder adopts it.
ALTER TABLE books ADD COLUMN directory_id INTEGER REFERENCES directories(id) ON DELETE CASCADE;

-- Without it, removing a folder reads every book to find its own.
CREATE INDEX books_by_directory ON books (directory_id);
