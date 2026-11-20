-- The direction a book's own file states, from a CBZ's or an image folder's ComicInfo.xml; NULL
-- when it states none.
ALTER TABLE books ADD COLUMN reading_direction TEXT CHECK (reading_direction IN ('rtl', 'ltr'));

-- A user's reading mode for a series, from their first choice on; until then the book's own
-- direction applies.
CREATE TABLE reading_modes (
    user_id INTEGER NOT NULL,
    series_id INTEGER NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('rtl', 'ltr', 'continuous')),
    PRIMARY KEY (user_id, series_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (series_id) REFERENCES series(id) ON DELETE CASCADE
);
