# Manga metadata

State: in progress

Manga libraries get their metadata from AniList, MangaDex and MangaUpdates, through the
[metadata enrichment](09-metadata-enrichment.md) flow.

## Build
- [ ] 1 h · `LibraryType::Manga`, sent and stored as `"manga"` — unknown strings still map to `Others`, so no migration is needed [BE]
- [ ] 1 h · Manga in the app's library type list [FE]
- [ ] 5 h · AniList adapter for `MetadataProvider`: GraphQL with constant query text and the search in `variables`; candidates for a title and one entry by id; descriptions as plain text; one request every 2 s, AniList's documented limit while degraded (30 a minute) [BE]
- [ ] 4 h · MangaDex adapter for `MetadataProvider` [BE]
- [ ] 4 h · MangaUpdates adapter for `MetadataProvider` [BE]
