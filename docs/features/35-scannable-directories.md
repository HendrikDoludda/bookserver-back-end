# Scannable directories

State: completed

The folders the scanner reads live in the database, managed through the API from both UIs, so
changing them needs no restart. A directory added with its "create a library" toggle on gets a
library of its own; with it off, its series wait in no library until an admin adds them to one.

## Build
- [x] 1 h · `directories` table: the canonical path (unique), `library_id` → `library` `ON DELETE SET NULL`, `created_at`; `books.directory_id` → `directories` `ON DELETE CASCADE` — migration 0006 [DB]
- [x] 3 h · Endpoints to add, list and remove scannable directories: `POST`, `GET` and `DELETE /directories`, admin only; their answers carry the directory's path [BE]
- [x] 1 h · Refuse a path that isn't an existing folder (400 `directory_not_found`), one already added (409 `directory_taken`), and one inside or around another directory, the cover folder or the database's folder (409 `directory_overlaps`) — today a missing `BOOK_DIRS` entry still gets a library row before its scan fails [BE]
- [x] 1 h · The toggle: with `create_library`, adding a directory creates a library named after the folder, or the given name, with the type the request names — required then; today every auto-created library is hard-coded to `LibraryType::Books` and named after the lowercased path [BE]
- [x] 2 h · Adding a directory adopts the stored books without a directory that lie inside it — compared canonical, since `BOOK_DIRS` could be relative — and with the toggle on, their series join its library [BE]
- [x] 1 h · Removing a directory deletes its books' rows and the series left without books; the files stay [BE]
- [x] 2 h · Scan the directories from the table and stop reading `BOOK_DIRS`; a toggle-on directory's new series join its library [BE]
- [x] 2 h · Scan one directory: `POST /directories/{id}/scan` (answers 501 until then), through [34](34-directory-scanning.md)'s scan job; adding a directory queues its first scan [BE]
- [x] 3 h · Add and remove directories in the app's Manage screen: the server path, the toggle with the library's name and type, and a confirmation before removing [FE]
