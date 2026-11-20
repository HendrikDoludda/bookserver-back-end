# Reader view settings

State: in progress

A font size for flowing EPUBs, and a zoom dropdown with set steps in the PDF, comic and
fixed-layout EPUB readers. Fixed-layout pages keep the sizes their files give, so they get the
zoom, not a font size.

## Build
- [x] 0.5 h · Texts in the `reader` namespace — the font-size control, the zoom dropdown and each of its steps — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [ ] 2.5 h · The flowing EPUB reader can put any block at the top of the screen: its list starts at that block (`CustomScrollView` with a `center`), the blocks before it above, the scrollbar's estimate as before [FE]
- [ ] 2 h · Font-size control in the flowing EPUB reader's bar ([83](83-app-layout.md)): 10 % smaller or larger, from 80 to 200 %, scaling the text style the blocks and the scrollbar estimate share (`_Blocks`), with the block at the top of the screen kept in place [FE]
- [ ] 1 h · Keep the chosen font size across books and app starts, on the device in [79](79-handedness.md)'s `preferences.json` [FE]
- [ ] 1.5 h · Zoom dropdown in the PDF reader's bar: fit page, fit width, then 150, 200 and 300 % of fit page, each setting pdfrx's zoom (`PdfViewerController.setZoom`), fit page and fit width worked out from the page on screen and the window; the step holds while the book is open; pinching still zooms freely [FE]
- [ ] 2 h · The same dropdown in the bar of the readers built on the page turner — comics and image folders in the paged modes, fixed-layout EPUB pages: a chosen step holds as the pages turn until the book closes, and a pinch on top of it is undone by the next turn, as now [FE]
- [ ] 0.5 h · Check by hand on a phone and the Mac: the font size changes a novel's text, keeps the paragraph being read on screen and is still set after an app restart; each zoom step fits a PDF page, a comic page and a cookbook page as its name says, and holds as the pages turn
