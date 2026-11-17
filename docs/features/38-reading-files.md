# Reading files

State: in progress

`GET /books/{id}/file` streams a book's file with HTTP range support through `tower-http`'s
`ServeFile`, and `GET /books/{id}` returns its metadata (404 for an unknown id). PDF,
EPUB and CBZ files stream; image-folder and CBZ books are also served page by page
(`GET /books/{id}/pages`, `GET /books/{id}/pages/{number}`). The app reads CBZ and image-folder
books page by page; the PDF and EPUB readers don't exist yet.

## Build
- [x] 1.5 h · The book JSON (`GET /books/{id}`, `GET /series/{id}/books`) exposes Rust internals and server paths: `last_modified` is `{"secs_since_epoch":…,"nanos_since_epoch":0}`, `language` is `{"Other":"unknown"}` or `"English"`, `format` is `"Cbz"`, and `file_path`, `file_hash` and `cover_image` show the server's folders. Answer a `BookSummary` instead: the time as unix seconds, as every auth answer does; the language as its code (`BookLanguage::to_code`), null when unknown; the format as `pdf | epub | cbz | image_comic` (`BookFormat::as_str`); no `file_path`, `file_hash` or `cover_image` [BE]
- [x] 3 h · Serve image-folder books page by page — their stored `file_path` is the folder, so `ServeFile` has no file to stream. `GET /books/{id}/pages` lists the pages as `[{"number": 1}, …]` and `GET /books/{id}/pages/{number}` serves one through `ServeFile`, numbered from 1: the folder's image files (`jpg`, `jpeg`, `png`, `webp`, any case; names starting with `.` skipped) in natural order — one listing, which the cover picker uses too. A PDF or EPUB book, or a number out of range, answers 404 `not_found` [BE]
- [x] 2 h · Serve CBZ books page by page through the same two routes: the archive's image entries (`__MACOSX/` and names starting with `.` skipped) in the same order, each read whole in `spawn_blocking` up to a size cap and sent with its media type. The CBZ cover picker takes this listing's first page instead of the first entry in plain alphabetical order (`img10.jpg` before `img2.jpg`) [BE]
- [ ] 2 h · Give the reader each page's dimensions in the page list, so it can lay pages out before they load (TODO in `get_pages`) [BE]
- [ ] 0.5 h · Delete the unused `StreamReaderErrors` and `convert_file_path_to_blob` — leftovers from a hand-rolled range reader that `ServeFile` replaced [BE]
- [x] 0.5 h · Texts for the readers in a `reader` namespace — opening, failures, retry, "Page {page} of {count}" — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [x] 1.5 h · `ApiClient.download`: streams an answer into a file, the timeout covering the wait for the answer and each gap between chunks but not the whole download; a download that fails leaves no file [FE]
- [ ] 0.5 h · The PDF and EPUB readers download the book's file through the session into a fresh temp file, and delete it when they close [FE]
- [x] 5 h · Image-page reader for CBZ and image-folder books: the pages from the page list in a `PageView`, turned by swipe and the arrow keys, zoomable, the next page preloaded, each page decoded at most at twice the screen's pixel width [FE]
- [ ] 3 h · PDF reader: the downloaded file in pdfrx's `PdfViewer.file` [FE]
- [ ] 8 h · EPUB reader, our own renderer: `archive` reads the downloaded file, `xml` its container and the OPF's spine, `flutter_widget_from_html_core` shows each chapter's body in one scrolling list, with its images read from the archive [FE]
- [ ] 0.5 h · Once the app reads PDFs, end the early-development practice of editing migration `0001` and wiping the database: every later schema change is a new numbered migration, and the exception line in [ARCHITECTURE.md](../ARCHITECTURE.md) goes [DB]
