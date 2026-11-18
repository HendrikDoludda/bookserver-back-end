# Reader view settings

State: not started

A font size for flowing EPUBs, and a zoom dropdown with set steps in the PDF, comic and
fixed-layout EPUB readers. Fixed-layout pages keep the sizes their files give, so they get the
zoom, not a font size.

## Build
- [ ] 0.5 h · Texts in the `reader` namespace — the font-size control, the zoom dropdown and each of its steps — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [ ] 2 h · Font-size control in the flowing EPUB reader's bar ([83](83-app-layout.md)): one step smaller or larger within the limits, scaling the text style the blocks and the scrollbar estimate share (`_Blocks`), with the block at the top of the screen kept in place [FE]
- [ ] ? h · Keep the chosen font size across books and app starts [FE]
- [ ] 1.5 h · Zoom dropdown in the PDF reader's bar: each step sets pdfrx's zoom (`PdfViewerController.setZoom`), fit page and fit width worked out from the page on screen and the window; pinching still zooms freely [FE]
- [ ] 2 h · The same dropdown in the bar of the readers built on the page turner — comics, image folders, fixed-layout EPUB pages: a chosen step holds as the pages turn, and a pinch on top of it is undone by the next turn, as now [FE]
- [ ] 0.5 h · Check by hand on a phone and the Mac: the font size changes a novel's text, keeps the paragraph being read on screen and is still set after an app restart; each zoom step fits a PDF page, a comic page and a cookbook page as its name says, and holds as the pages turn

## Open decisions
- [ ] Which steps the font size and the zoom offer. Suggested: font size 80–200 % in 10 % steps; zoom fit page, fit width, then 150, 200 and 300 % of fit page — blocks: the font-size control, both zoom dropdowns
- [ ] Where the font size is kept: on the device, like [79](79-handedness.md)'s handedness, or per user in `user_settings` ([31](31-per-user-settings.md)), which follows to every device but needs a migration and a check in `PUT /api/v1/account/settings` — blocks: keeping the font size
- [ ] Whether a zoom step stays only while the book is open, or is kept like the font size — blocks: both zoom dropdowns
