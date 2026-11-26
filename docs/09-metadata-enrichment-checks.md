# 09 Metadata enrichment — hand checks

## Step 1 · Schema
- [ ] Stop the server and start it again with `cargo run`; the log shows `Applied migration 0011 metadata_enrichment` and no error
- [ ] `sqlite3 "$HOME/Library/Application Support/com.hendrikdoludda.bookserver/app_data.sqlite" "SELECT MAX(version) FROM schema_migrations; SELECT COUNT(*), TOTAL(is_sensitive) FROM series; SELECT COUNT(*), TOTAL(is_sensitive) FROM library;"` answers `11`, then your series count with `0.0`, then your library count with `0.0` — nothing is sensitive
- [ ] The app shows your libraries, series and books as before
