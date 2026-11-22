-- A file's size and modification time, or an image folder's digest of its pages': a scan compares
-- it to tell a changed file from an unchanged one without reading it. NULL until the book's file
-- is next read.
ALTER TABLE books ADD COLUMN fingerprint TEXT;
-- A scan finds a moved file's row by its hash.
CREATE INDEX books_by_hash ON books (file_hash);
