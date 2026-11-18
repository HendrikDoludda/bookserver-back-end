# Browsing

State: in progress

The app's way from the libraries to a book: a library's series, a series' books, and opening a
book in the [reader](38-reading-files.md) for its format.

## Build
- [x] 1.5 h · Reading order: `GET /libraries/{id}/series` lists series by name, case-insensitive; `GET /series/{id}/books` lists books by volume, then chapter — a missing number after the numbered ones — then title, then id. The batch read (`Extract::extract_batch`) returns rows in the order of the ids it is given [BE]
- [x] 1 h · The library handlers (`get_libraries`, `get_series_in_library`, `get_series_children`, `retrieve_metadata`, `request_file`, `delete_library`) reach the database through `Database::read` or `Database::transact`, not by calling the pooled `Database` methods from the async handler — each such call blocks a Tokio worker thread [BE]
- [x] 0.5 h · Texts for the browse screens in a `browse` namespace — empty lists, reload, the volume and chapter labels — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [x] 2 h · The libraries as a grid of cover cards — the card the series and books use too — replacing 73's placeholder home [FE]
- [x] 2 h · A library's series as a grid of covers, from `GET /api/v1/libraries/{id}/series` [FE]
- [x] 2 h · A series' books as a grid of covers, labelled by volume and chapter, from `GET /api/v1/series/{id}/books` [FE]
- [x] 2 h · Covers in the three lists from `GET /api/v1/covers/{kind}/{id}` ([36](36-covers.md)), loaded through the session: a placeholder while loading and when there is no cover [FE]
- [x] 1 h · Back navigation: each screen is pushed onto the navigator; when the session ends, the app shows the login screen with nothing left on top [FE]
- [x] 0.5 h · Open a book in the reader for its format; a format the app can't read says so [FE]
- [x] 2 h · The library and series lists send a count: each library its number of series (`GET /libraries`), each series its number of books (`GET /libraries/{id}/series`) [BE]
- [x] 0.5 h · Texts for those counts in the `browse` namespace, one key per plural form, each form with `{count}` — the one-form too ("{count} series" / "{count} series", "{count} book" / "{count} books") — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [ ] 1 h · The library and series cards show that count under their name, in the language's plural form, picked by [57](57-frontend-localization.md)'s plural task [FE]
- [ ] 0.5 h · `ServerImage` takes a failed load out of the image cache, as Flutter's `NetworkImage` and `ResizeImage` do — today only the `ResizeImage` around every use does, so a bare `ServerImage` that failed once would never load again until the app restarts [FE]

