# Merge series

State: not started

An admin merges series A into series B: A's books, library places and reading modes move to B, A
is deleted, and the scanner files A's future books into B. It fixes any grouping the scanner got
wrong — one series split by two naming styles, or by a misspelt file name.

## Build
- [ ] 1.5 h · Migration `0009`: a `series_match_keys` table — `series_id` (→ `series`, `ON DELETE CASCADE`), `match_key`, primary key on both, an index on `match_key` — holding the keys a series answers to besides its own [DB]
- [ ] 1.5 h · `series_of_directory_by_match_key_with_connection` also finds a series by a key in `series_match_keys`, so `series_for` puts a new file named like A into B [DB] [BE]
- [ ] 4 h · `merge_series`, in one transaction: A's books go to B; B joins every library A is in; a user's reading mode for A moves to B unless they have one for B; B keeps its name, and its description, cover and years, taking A's for the empty ones; A's `match_key` and extra keys become B's extra keys; A is deleted [BE]
- [ ] 1 h · `POST /api/v1/series/{id}/merge` with `{"into"}`, behind `AdminSession`, answering B as `PATCH /series/{id}` does; merging a series into itself is a 400, an unknown id a 404 [BE]
- [ ] 3 h · "Merge into…" on the series edit screen: pick B from `GET /series` — with book count and libraries, so two series of one name can be told apart — confirm, then show B [FE]
- [ ] 0.5 h · Check by hand: merge two series of one folder, then add a new volume named like the merged-away one; after a scan it shows in the kept series
