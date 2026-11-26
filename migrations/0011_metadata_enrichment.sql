-- A sensitive library or series is never looked up online.
ALTER TABLE library ADD COLUMN is_sensitive BOOLEAN NOT NULL DEFAULT FALSE CHECK (is_sensitive IN (0, 1));
ALTER TABLE series ADD COLUMN is_sensitive BOOLEAN NOT NULL DEFAULT FALSE CHECK (is_sensitive IN (0, 1));

-- The provider entry a series was matched to; a series without a row was never matched.
CREATE TABLE series_metadata_sources (
    series_id INTEGER PRIMARY KEY REFERENCES series(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_id TEXT NOT NULL,
    matched_by TEXT NOT NULL CHECK (matched_by IN ('auto', 'admin')),
    matched_at INTEGER NOT NULL
);

-- A provider's answer to one request — a search or one entry — kept so it isn't asked again.
CREATE TABLE metadata_cache (
    provider TEXT NOT NULL,
    request_key TEXT NOT NULL,
    response TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    PRIMARY KEY (provider, request_key)
);
