# Reading files — checks

First: the server as it runs now, with your demo folder in `BOOK_DIRS` ([`run`](run-deploy/local.md#commands)) — nothing on the server changed. Then `cd ~/bookserver-front-end && flutter run -d macos`; if macOS asks whether "bookserver" may use your confidential information, enter your Mac password and click Always Allow — Deny signs the app out and proves nothing.

## macOS
- [x] Step 8 · In the demo library, open the series "bobby make believe sample" and its book: "bobby make believe sample" in the title bar, the first page, and "Page 1 of 4" under it
- [x] Step 8 · → shows "Page 2 of 4", ← "Page 1 of 4" again; on page 1, ← does nothing, and on page 4, → does nothing
- [x] Step 8 · A two-finger swipe to the left on the trackpad turns to the next page, one to the right back
- [x] Step 8 · Pinch outwards on the trackpad: the page grows, up to about 2.5 times; while it's zoomed, a two-finger swipe moves around the page instead of turning it; then → shows the next page at normal size
- [x] Step 8 · "bobby make believe sample dir", whose pages lie in a folder inside the archive, reads the same as "bobby make believe sample"
- [x] Step 8 · "picture book": "Page 1 of 18", page 1 is the fern leaf that is also its cover, and the pages follow the file names in `~/Documents/BookserverTest/Picture Book C1` sorted by name, in Finder
- [x] Step 8 · On page 1 of "picture book", stop the server (Ctrl-C in its terminal), then press → twice: "Page 3 of 18" says "This page couldn't be loaded." with "Try again" under it. Start the server again ([`run`](run-deploy/local.md#commands)), click "Try again": the page shows
- [x] Step 8 · Open "picture book" and leave it on a page for 16 minutes — the session token lasts 15 — then press → three times: each page shows. `sqlite3 ~/bookserver_backend/data/databases/app_data.sqlite "SELECT datetime(session_token_valid_until, 'unixepoch', 'localtime') FROM sessions WHERE device_name = 'macOS'"` then shows a time about 15 minutes after your key presses: the app renewed its session on the way
- [x] Step 8 · The back arrow at the top left returns to the series' books

## At the end of the plan: iOS simulator and Android emulator
- [ ] Step 8 · Open "bobby make believe sample": a swipe to the left turns to the next page, to the right back; pinch to zoom (in the iOS simulator, hold ⌥ and drag); while zoomed, a drag moves around the page; pinch back to normal size, and a swipe turns the page again
