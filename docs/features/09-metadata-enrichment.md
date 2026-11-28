# Metadata enrichment

State: in progress

Fill in what filenames can't tell — descriptions, covers, tags, release years, authors — from
online databases. Each library type has one provider; a title search returns candidates, and only
an admin's confirmed candidate or a single exact match is merged into the series. The adapters are
[63](63-book-metadata.md) books, [64](64-manga-metadata.md) manga and [65](65-comic-metadata.md)
comics. A sensitive item ([10](10-sensitive-content-controls.md)) never goes online and nothing
overrides that: its metadata is entered by hand ([66](66-manual-metadata-editing.md)).

## Build
- [x] 2 h · Migration 0011: `is_sensitive` (`BOOLEAN NOT NULL DEFAULT FALSE`) on `library` and `series`, in both models' field, `Extract::COLUMNS`, `FromRow` and `Insert`; tables `series_metadata_sources` (series, provider, external id, matched by `auto` or `admin`, when) and `metadata_cache` [DB]
- [x] 1 h · A series is sensitive when its own flag or any of its libraries' flag is set — one query [DB]
- [x] 3 h · `src/metadata/`: `MetadataProvider`, one per library type through `provider_for` (Manga → AniList); `clear` is the only way to an adapter call — it refuses a sensitive series (`sensitive_item`) and picks the provider from the series' libraries (`no_metadata_provider`) [BE]
- [x] 2 h · A library's `adult_metadata` toggle (migration 0012, off by default), set through `PATCH /libraries/{id}` and sent in every library answer; a series' lookups include entries a provider marks adult when any of its libraries has it on (`ClearedSeries::includes_adult`) [DB] [BE]
- [ ] 1 h · Once a second provider exists: a series whose libraries' types have different providers answers 409 `ambiguous_metadata_provider` from `clear` — until then `clear` takes the first [BE]
- [x] 1 h · HTTP client: `reqwest` with `rustls`, as [52](52-packaging-and-deployment.md) will serve TLS; one client, no redirects, timeouts, capped bodies [BE]
- [x] 2 h · Each provider's documented rate limit (not one global number); a 429's `Retry-After` is honoured; an admin's request that would wait over 10 s answers 503 `provider_rate_limited` with `Retry-After` [BE]
- [x] 3 h · Cache provider answers in `metadata_cache` for 30 days, keyed by provider, kind of lookup, whether it includes adult entries, and the normalised query or the external id; a failure isn't kept [DB] [BE]
- [x] 2 h · `GET /series/{id}/metadata/candidates?query=`, admin only: the provider's candidates for the query (default: the series' name; trimmed, 1–200 characters, else 400 `invalid_query`), each marked `exact_match` when its title or one of its other titles equals the query as `match_key` normalises both; changes no series [BE]
- [ ] 3 h · Merge a candidate into a series in one transaction: description and release years on the series, author and tags on its books, and the source row. Replace mode writes every field the candidate has; fill mode only empty ones [BE]
- [ ] 1 h · In fill mode `"Unknown Author"` and no tags count as empty; the language is never merged [BE]
- [ ] 1 h · Tags merge through one split, de-duplicate and re-join helper over the comma-joined `books.tags`; a comma inside a tag becomes a space [BE]
- [ ] 1 h · `PUT /series/{id}/metadata_source` `{"external_id"}`, admin only: fetches the entry itself and merges it in replace mode [BE]
- [ ] 2 h · A candidate's cover is fetched over https from its provider's image host only, capped at 10 MB, checked to be a JPEG, PNG or WebP, and written to the cover folder under its hash; replace mode sets the series' cover, fill mode only when the series shows none [BE]
- [ ] 4 h · Bulk pass in the background, one at a time, admin only: `POST /metadata/scan` (every library) and `POST /libraries/{id}/metadata/scan`; only series never matched; expired cache rows deleted at its start (`db_metadata::delete_expired`); 409 `metadata_scan_in_progress` [BE]
- [ ] 1 h · The bulk pass auto-matches only a single candidate that `Candidate::is_exact_match` the series' name — its title or one of its other titles; fill mode; every auto-match is logged [BE]
- [ ] 1 h · `GET /metadata/scan/status`, admin only: the running pass's counts and how the last one ended [BE]
- [ ] 2 h · Provider API keys in the database, encrypted with XChaCha20-Poly1305 under a key BLAKE3-derived from the signing key in `secrets.toml` (own context string, the provider's name as associated data, as `TotpCipher` does); write-only — an admin endpoint sets or replaces a key and answers only whether one is set; a key is never logged. Built with the first adapter that needs a key ([63](63-book-metadata.md), [65](65-comic-metadata.md)) [DB] [BE]
- [ ] 4 h · Candidate list and confirm screen, reached from the admin's series form [FE]
- [ ] 1 h · "Fetch metadata" on the admin's library form, starting the library's bulk pass [FE]
- [ ] 0.5 h · "Search adult entries" switch on the admin's library form, editing only [FE]
