# Covers

State: in progress

The scanner extracts one cover per book into `./cover_images/<file hash>.<ext>` — the first
page of a PDF, the first image of a CBZ or image folder, an image from an EPUB.
`GET /covers/{kind}/{id}` serves a book's cover, and a series' or library's: its own — which
nothing sets yet — else its first book's.

## Build
- [ ] 2 h · Without the PDFium library, PDFs are left out of the catalog entirely: `Pdfium::default()` panics during cover extraction, which runs before the book row is inserted. Bind once at start with the calls that return an error, and hand the scan the library as an `Option`, so a missing library costs the cover, not the book [BE]
- [x] 2 h · Serve covers at `GET /covers/{kind}/{id}` (answers 501 until then) through `ServeFile`, the way `request_file` streams books. `kind` is `book`, `series` or `library` — any other answers 400; a row without a cover, or a cover file gone from disk, answers 404 `not_found` as JSON [BE]
- [x] 1.5 h · Series and library covers fall back when read, nothing stored: a row's own `cover_image` first; else, for a series, its first book with a cover in reading order ([43](43-browsing.md)); for a library, the cover of its first series by name that has one [BE]
- [ ] 3 h · An admin chooses a series' cover from its books' covers and a library's from its series': `PUT` and `DELETE /series/{id}/cover` and `/libraries/{id}/cover`, where `DELETE` goes back to the automatic cover — the columns exist, but nothing sets them [BE]
- [ ] 0.5 h · The series and library JSON stop sending `cover_image`, a server file path — the app gets covers from `GET /covers/{kind}/{id}` [BE]
- [ ] 3 h · Choose a series' or a library's cover in the app's Manage screen, and drop the app's cached covers after a change [FE]
- [ ] 3 h · EPUB: take the cover the package document names — EPUB 3's `cover-image` item, else EPUB 2's `<meta name="cover">`, else an image whose id says cover — instead of `epub.images.first()`, reading only that entry with `zip` and `quick-xml`; the extension comes from a known media type (`image/svg+xml` becomes `.svg+xml` today), and any other gets no cover. `epub-parser`, used only here, goes [BE]
- [ ] 1 h · When hashing a file fails, its cover is written to `./cover_images.jpg` — one file beside the cover folder, shared by every such book — because the empty hash is joined to the folder path and `with_extension` renames the folder. Skip the cover when there is no hash [BE]
- [ ] 1 h · Check for an existing cover before extracting: CBZ never checks, and PDF renders the page before it checks [BE]
- [ ] 1 h · The cover folder is a value in `AppState` that the scanner and the cover route take, `./cover_images/` by default, instead of a constant in the working directory, so a test of the scanner doesn't write covers where it was started; rows store the bare file name, and migration 0004 strips the prefix from existing rows. Where the folder lives is [52](52-packaging-and-deployment.md)'s [DB] [BE]
- [ ] 0.5 h · Check that an image folder's first file is an image (`image::guess_format`, as for a CBZ) before copying it as the cover [BE]
- [ ] 0.5 h · Check by hand: the cover extracted from a real `.epub` is the book's actual cover

## Missing information
- [ ] Can one bound `Pdfium` be shared between threads with the `thread_safe` feature? — answered by: pdfium-render's source, in the binding task
