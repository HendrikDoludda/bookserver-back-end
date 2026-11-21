# Checks — 36 Covers

In the app's debug build on this Mac, signed in as the admin.

- [ ] Stop the server and start it again with [`run`](run-deploy/local.md#commands) — the new texts are compiled into this build.
- [ ] Start the app from the `bookserver-front-end` folder with [`run-macos`](../../bookserver-front-end/docs/run-deploy/local.md#commands). A new build asks for the keychain, where the session is kept: enter your Mac password and click Always Allow.
- [ ] Open a library whose series grid shows a series with several books. From the account menu there, open Manage → Series → that series: the form has a **Cover** row showing the series' cover.
- [ ] Tap Cover: "Choose a cover" shows **Automatic** and every book of the series. Tap a book other than the first — the form's Cover row now shows that book, and nothing is saved yet. Save, then go Back until the series grid: without Reload or a restart, the series shows that book's cover.
- [ ] The same series → Cover → **Automatic** → Save, then Back to the series grid: the series shows its first book's cover again.
- [ ] Manage → Libraries → that library → Cover: only the series ticked in the form are offered. Pick one, Save, then Back to the home screen: the library shows that series' cover. Then Cover → Automatic → Save: the library shows its automatic cover again.
- [ ] Manage → Series → the series of one of your two EPUBs without a cover → Cover: that book's card is greyed out, and tapping it does nothing.
- [ ] In Settings choose Deutsch: the row says "Titelbild", the grid "Titelbild wählen" and "Automatisch". Switch back afterwards.
