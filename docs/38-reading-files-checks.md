# Reading files — checks

First: the server running with your demo folder in `BOOK_DIRS` ([`run`](run-deploy/local.md#commands)) — nothing on the server changed. For the Step 9 checks, put one omnibus PDF into the demo folder and scan: [`login`](run-deploy/local.md#commands) as the admin, then [`scan`](run-deploy/local.md#commands) with the `session_token` from the login's answer. Then `cd ~/bookserver-front-end && flutter run -d macos` — the first build since pdfrx downloads PDFium and takes longer; if macOS asks whether "bookserver" may use your confidential information, enter your Mac password and click Always Allow — Deny signs the app out and proves nothing.

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
- [x] Step 9 · Open "A throne stolen" Volume 1: "A throne stolen" in the title bar, its pages one under the other, and "Page 1 of" its page count under them; scrolling down counts up
- [x] Step 9 · Pinch outwards on the trackpad: the page grows, and its lines and letters stay sharp, not blurry. Page Down and Space (fn+↓) go to the next page, Page Up (fn+↑) back, End (fn+→) to the last page, Home (fn+←) to the first
- [x] Step 9 · Open the omnibus: page 1 shows within a few seconds, not after the whole file has arrived. While it's open, `find ~/Library/Containers/com.hendrikdoludda.bookserver -maxdepth 6 -type d -name pdfrx.cache -exec du -sh {} \;` shows a few megabytes, far below the omnibus's size; scroll on a few pages and run it again: it has grown by roughly a page's size per page
- [x] Step 9 · Back to the books, then the same `find` command prints nothing: the downloaded blocks are gone
- [x] Step 9 · Stop the server, then open a PDF: "This book can't be opened." with "Try again" under it. Start the server again, click "Try again": the book opens
- [x] Step 9 · With a PDF open, stop the server and scroll to pages you haven't seen yet: say what they show
- [x] Step 9 · Then start the server again ([`run`](run-deploy/local.md#commands)) — new pages load with their pictures (reported) — and scroll back up to the pages that showed only text or stayed white while the server was off: say whether they now show in full — the plan leaves this open
- [x] Step 9 · Open a PDF and leave it for 16 minutes, then press End (fn+→): the last page shows, and `sqlite3 ~/bookserver_backend/data/databases/app_data.sqlite "SELECT datetime(session_token_valid_until, 'unixepoch', 'localtime') FROM sessions WHERE device_name = 'macOS'"` shows a time about 15 minutes after the key press
- [x] Step 10 · If the cookbook `goodforyou.epub` isn't in the catalog yet, scan once more ([`login`](run-deploy/local.md#commands), then [`scan`](run-deploy/local.md#commands)). Then open "fitzgerald great gatsby" and its book: "Opening the book…" with a bar, then the contents page and the chapters one after the other, in a column in the middle of the window; make the window narrow, and the text wraps to it
- [x] Step 10 · "plath bell jar" reads the same way, and so do "test book epub" and "test book epub more detail"
- [x] Step 10 · Close a book with the back arrow, then `find ~/Library/Containers/com.hendrikdoludda.bookserver -maxdepth 6 -name 'book-*.epub'` prints nothing: the downloaded file is gone
- [x] Step 10 · Open "goodforyou": the bar fills as its 352 MB arrive, then its text shows as loose lines without photos — its pages come with the next steps
- [x] Step 10 · Stop the server, then open an EPUB: "Can't reach the server…" with "Try again" under it; start the server again and click "Try again": the book opens
- [x] Step 10b · Open "bobby make believe sample": it reads exactly as in the Step 8 checks above — → and ← turn the pages, never past either end; a two-finger swipe turns them too; pinch to zoom, then a swipe moves around the page; → shows the next page at normal size

## At the end of the plan: iOS simulator and Android emulator
- [ ] Step 8 · Open "bobby make believe sample": a swipe to the left turns to the next page, to the right back; pinch to zoom (in the iOS simulator, hold ⌥ and drag); while zoomed, a drag moves around the page; pinch back to normal size, and a swipe turns the page again
- [ ] Step 9 · Open "A throne stolen" Volume 1: its pages show, scrolling and pinch-zoom work. The first Android build compiles a small C library for pdfrx and needs the Android NDK, which Gradle should install by itself: if the build stops, paste its error
