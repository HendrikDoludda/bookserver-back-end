# Manual metadata editing

State: not started

An admin can edit any metadata field by hand — to fix a wrong automatic match, or to describe a
sensitive item that must never be looked up online. A later enrichment run never overwrites a
hand-edited field.

## Build
- [ ] 2 h · Endpoint to edit a book's metadata fields, behind `AdminSession` — library and series editing live in `library_management.rs` [BE]
- [ ] 3 h · Remember which fields were edited by hand (a per-field marker), and skip them in every later merge and in the scan's re-parse of the file name (`folder_scanner.rs`) [DB] [BE]
- [ ] 4 h · Edit screen for any metadata field of a book or series [FE]
