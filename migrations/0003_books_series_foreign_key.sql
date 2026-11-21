-- `books.series` becomes a declared foreign key. SQLite can't add one to an existing column, so
-- the table is rebuilt: create the new one, copy every row with its id, drop the old one, rename.
-- The runner keeps foreign keys on, so this is safe only while no other table references `books`.
CREATE TABLE books_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    author TEXT NOT NULL,
    format TEXT NOT NULL,
    language TEXT NOT NULL,
    cover_image TEXT,
    description TEXT,
    tags TEXT NOT NULL,
    file_path TEXT NOT NULL UNIQUE,
    page_count INTEGER,
    volume_number INTEGER,
    -- NUMERIC keeps a whole number an integer and stores a decimal chapter such as 1.5 as one.
    chapter_number NUMERIC,
    page_number INTEGER,
    file_hash TEXT,
    last_modified INTEGER,
    file_size INTEGER,
    -- A series with books can't be deleted; a library only unlinks its series.
    series INTEGER REFERENCES series(id) ON DELETE RESTRICT,
    reading_direction TEXT CHECK (reading_direction IN ('rtl', 'ltr'))
);

-- A series id that names no series would fail the copy, and the server would no longer start.
INSERT INTO books_new (id, title, author, format, language, cover_image, description, tags,
    file_path, page_count, volume_number, chapter_number, page_number, file_hash, last_modified,
    file_size, series, reading_direction)
SELECT id, title, author, format, language, cover_image, description, tags,
    file_path, page_count, volume_number, chapter_number, page_number, file_hash, last_modified,
    file_size, (SELECT series.id FROM series WHERE series.id = books.series), reading_direction
FROM books;

-- AUTOINCREMENT's counter, so the id of a book deleted from the top is never handed out again.
DELETE FROM sqlite_sequence WHERE name = 'books_new';
INSERT INTO sqlite_sequence (name, seq) SELECT 'books_new', seq FROM sqlite_sequence WHERE name = 'books';

DROP TABLE books;
ALTER TABLE books_new RENAME TO books;

CREATE INDEX books_by_series ON books (series);
