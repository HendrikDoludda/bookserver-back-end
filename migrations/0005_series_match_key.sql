-- The scanner finds a series by this key, never by its name, which an admin can change. Rust
-- lowercases it: SQLite's lower() folds only ASCII letters.
ALTER TABLE series ADD COLUMN match_key TEXT NOT NULL DEFAULT '';

-- Until now the name was the key, lowercased by Rust. lower() also recovers the key of a series
-- whose capitals an admin changed through the API before this migration.
UPDATE series SET match_key = lower(name);

-- A scanned name is its first book's title, lowercased; the title still has its capitals. A name
-- that no title lowers to, such as one an admin chose, stays as it is.
UPDATE series SET name = (
    SELECT title FROM books
    WHERE books.series = series.id AND lower(books.title) = series.name
    ORDER BY books.id LIMIT 1
)
WHERE EXISTS (
    SELECT 1 FROM books WHERE books.series = series.id AND lower(books.title) = series.name
);

CREATE INDEX series_by_match_key ON series (match_key);
