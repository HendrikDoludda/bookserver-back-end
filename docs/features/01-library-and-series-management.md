# Library & series management

State: in progress

Libraries are made by hand or by a directory's toggle ([35](35-scannable-directories.md)); this
makes libraries and series manageable — create a library and choose its series, edit and delete
both — through the same API for the app and the admin view. Deleting a library only unlinks it:
its series and books stay.

## Build
- [x] 0.5 h · Send `library_type` in snake_case (`books`), like every other enum — the library JSON sends the Rust variant name `"Books"` today [BE]
- [x] 0.5 h · `DELETE /libraries/{id}` answers 200 "Library deleted" for an id no library has — `remove_entry` returns the deleted-row count; answer 404 `not_found` when it is 0, and 204 instead of the text otherwise [BE]
- [x] 0.5 h · Delete the unused `GetLibraryRequest` [BE]
- [x] 2.5 h · Declare `books.series` a foreign key to `series(id)`, `ON DELETE RESTRICT`, with an index: migration 0003 rebuilds `books` — create, copy the rows with their ids and `sqlite_sequence`'s counter, drop, rename — since SQLite can't add one to an existing column; a `series` naming no series becomes NULL, and `chapter_number` is declared `NUMERIC` for [34](34-directory-scanning.md)'s decimal chapters [DB]
- [x] 3 h · Create a library with its name, type and description: `POST /libraries`, which answers 501 until then [BE]
- [x] 2 h · Edit a library's name, type and description: `PATCH /libraries/{id}`, which answers 501 until then; a field sent as `null` is cleared, an absent one kept [BE]
- [x] 1 h · List every series with its book count and the libraries it is in — `GET /series` — to choose a library's series from [BE]
- [x] 1 h · Add a series to a library and remove it: `PUT` and `DELETE /libraries/{id}/series/{series_id}` [BE]
- [x] 2 h · Edit a series' name, description and release years: `PATCH /series/{id}` — no route exists; the name shown changes, the `match_key` the scanner matches by ([34](34-directory-scanning.md)) stays [BE]
- [x] 1 h · Delete a series, only an empty one: `DELETE /series/{id}` — no route exists; 409 `series_not_empty` while it has books [BE]
- [x] 0.5 h · Test that deleting a library only unlinks it: its series and books stay [BE]
- [x] 1 h · `GET /account`: the signed-in user's own summary with the role, so the app shows its admin screens only to admins [BE]
- [x] 1 h · The app asks `GET /account` whenever a session starts and shows **Manage** in the user menu only to admins [FE]
- [x] 3 h · Create a library and choose its series in the app's Manage screen [FE]
- [x] 3 h · Edit and delete libraries and series in the app's Manage screen [FE]
