# Table of contents

State: not started

A contents button in the EPUB and PDF readers lists the book's own table of contents and goes
to the chosen entry. A book without one shows no button.

## Build
- [ ] 0.5 h · Texts in the `reader` namespace — the contents button and the list's title — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [ ] 2 h · Read an EPUB's table of contents in `epub_book.dart`: the EPUB 3 navigation file (the manifest item whose `properties` include `nav`) and its `<nav epub:type="toc">` list, else the EPUB 2 NCX (the manifest item the spine's `toc` attribute names) and its `navPoint`s — each entry's title, file and anchor, and the entries under it; a book with neither has none [FE]
- [ ] 1.5 h · Contents list shared by the EPUB and PDF readers: the button, in the reader bar ([83](83-app-layout.md)) and only when the book has entries, opens the entries indented by level; choosing one closes the list and goes there [FE]
- [ ] 1.5 h · Going to an entry in the flowing EPUB reader: the first block at or after its anchor, else its file's first block [FE]
- [ ] 0.5 h · Going to an entry in the fixed-layout EPUB reader: the page made from its file, through `PageTurner.turnTo` [FE]
- [ ] 1.5 h · The PDF outline in the contents list: pdfrx's `PdfDocument.loadOutline()` — each node's title and children — and choosing one goes to its `PdfDest` (`PdfViewerController.goToDest`) [FE]
- [ ] 0.5 h · Check by hand: a novel's contents go to the right chapter, the cookbook's to the right recipe page and a PDF's outline to the right page; a book without contents shows no button
