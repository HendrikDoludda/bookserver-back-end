-- The keys a series answers to besides its own `match_key`: those of the series merged into it.
CREATE TABLE series_match_keys (
    series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    match_key TEXT NOT NULL,
    PRIMARY KEY (series_id, match_key)
);

-- The scanner looks a key up for every new book, and the primary key starts with the series.
CREATE INDEX series_match_keys_by_key ON series_match_keys (match_key);
