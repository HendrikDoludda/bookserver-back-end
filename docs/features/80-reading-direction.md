# Reading direction

State: in progress

A button in the PDF, CBZ and image-folder readers switches between right to left, left to right
and continuous scroll. Each user's choice is kept per series on the server; until there is one,
a book opens in the direction its file states, else continuous for a PDF and left to right for
the rest. EPUB lays out its own text and gets no button; [handedness](79-handedness.md) only
moves where the button sits.

## Build
- [ ] 1 h · Table of each user's reading mode per series: `(user_id, series_id)` as primary key, both declared foreign keys with `ON DELETE CASCADE`, the mode stored as text from a Rust enum (`rtl` / `ltr` / `continuous`) [DB]
- [ ] 0.5 h · Nullable `books.reading_direction` (`rtl` / `ltr`): the direction the file states, NULL when it states none [DB]
- [ ] 2 h · Scanner fills `books.reading_direction` from a `ComicInfo.xml` — inside a CBZ, or in an image folder: `<Manga>YesAndRightToLeft</Manga>` is `rtl`, `<Manga>No</Manga>` is `ltr` [BE]
- [ ] 1 h · `GET /books/{id}` answers the mode the calling user opens it in: their choice for the book's series, else `books.reading_direction`, else `continuous` for a PDF and `ltr` for the rest [BE]
- [ ] 1 h · `PUT /api/v1/series/{id}/reading_mode` with `{"mode": …}` stores the calling user's choice and answers the stored `{"mode": …}`; an unknown mode answers 400 `invalid_reading_mode` (its text in `errors`, en and de), an unknown series 404 `not_found` [BE]
- [x] 0.5 h · Texts: the button, the three modes and a failed save in the `reader` namespace, in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [ ] 1 h · The PDF, CBZ and image-folder readers open in the mode `GET /books/{id}` answers [FE]
- [ ] 1 h · Reading-mode button in the reader bar ([83](83-app-layout.md)) of the PDF, CBZ and image-folder readers: switches at once and saves the choice with the `PUT` [FE]
- [ ] 2 h · Right to left and left to right in the comic readers (the page turner): page order, swipe direction and arrow keys follow the mode [FE]
- [ ] 3 h · PDF paged modes: pdfrx's `PdfPaging`, pages side by side fitted to the window — right to left through a mirrored layout — and the arrow keys turn pages [FE]
- [ ] 2.5 h · Each page's width and height in the page list (`GET /books/{id}/pages`), read from each image's header rather than the whole file with `imagesize` 0.15, so CBZ and image-folder continuous scroll lays pages out before they load (TODO in `get_pages`); null when unreadable; EXIF rotation not applied [BE]
- [ ] 2 h · Continuous scroll for CBZ and image-folder books, as the PDF reader already scrolls: the pages run top to bottom in one scrolling column, with no zoom [FE]
- [ ] 2 h · Page-turn tap zones as one layout shared by the readers; in the two paged modes the "next page" edge follows the mode; the middle still shows and hides the reader bar [FE]
- [ ] 0.5 h · Check by hand: a right-to-left manga goes forward by swiping right and tapping the left edge; the chosen mode carries over to the next volume and to another device
