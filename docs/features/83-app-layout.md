# App layout

State: in progress

What the screens share: a user menu at the top right of every browse screen, a sidebar for
switching libraries from inside one, and a bar for the readers' controls that hides while
reading. Colours and style are [08](08-theming-and-branding.md)'s.

## Build
- [x] 0.5 h · Texts in a `menu` namespace — the user button, Settings, Sign out (moved from `home.sign_out`) and the sidebar toggle — in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [x] 1 h · User menu: a user icon at the top right of every browse screen — the library grid, a library's series, a series' books — with Sign out; the sign-out and the temporary settings buttons in the library grid's top bar go [FE]
- [x] 0.5 h · Settings in the user menu opens [58](58-account-screens.md)'s settings screen [FE]
- [x] 3 h · Library sidebar inside a library — its series and a series' books: the libraries by name, the current one marked; a toggle in the top bar shows and hides it, beside the content in a wide window and over it in a narrow one, where a close button at the top of the list closes it; choosing a library shows its series in place of the current library's screens, so Back still leads to the library grid [FE]
- [ ] 2.5 h · Reader bar shared by the readers, replacing each reader's own top bar and lying over the page: on macOS, Windows and Linux, Back, the title and the controls in one bar at the top; on phones and tablets, Back and the title at the top and the controls at the bottom, once a reader has controls; "Page X of Y" stays under the pages. Shown when a book opens, hidden once the page turns or the text scrolls, and shown or hidden by a tap in the middle of the page that isn't on a link [FE]
- [ ] 0.5 h · Check by hand on a phone and the Mac: the user menu signs out; inside a library the sidebar switches to another one and Back leads to the library grid; in each reader a tap in the middle shows and hides the bar, and a link still works
