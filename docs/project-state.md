# Project state

Released: the server as `v0.1.4` and the app as preview builds, both on GitHub.

## Server

Every endpoint except the health check, the auth routes and the translation catalog needs a
logged-in user.

- **Downloads** for macOS (Intel and Apple Silicon), Windows and Linux on the Releases page, with PDFium and a `README.txt` on starting it; everything the server writes lies in one data folder.
- **Book folders** (`/api/v1/directories`, admin only): adding a folder, with a library of its own or none, scans it; removing one takes its books out of the catalog and leaves the files.
- **Scanning** in the background, one scan at a time (`POST /api/v1/scan`, `POST /api/v1/directories/{id}/scan`, `GET /api/v1/scan/status`, admin only): PDF, EPUB, CBZ and image-folder books become book and series rows, with volume, decimal chapter and page from the file names — titles matched regardless of case or accent encoding — and one cover each, PDFs only with the PDFium library. Only changed files are read again, a moved or renamed file keeps its row, a deleted one leaves the catalog — never while its folder is unavailable —, and what couldn't be read is reported.
- **Watching**: a change in an added folder is scanned by itself, about 10 seconds after the last one; a drive plugged back in is watched and scanned again.
- **Streaming** a book file with HTTP range requests (`GET /api/v1/books/{id}/file`) — only a file that lies inside its folder — and the pages of CBZ and image-folder books one by one (`GET /api/v1/books/{id}/pages…`).
- **Browsing**: the libraries (`GET /api/v1/libraries`), a library's series (`GET /api/v1/libraries/{id}/series`), a series' books in reading order (`GET /api/v1/series/{id}/books`), one book's metadata (`GET /api/v1/books/{id}`) and covers (`GET /api/v1/covers/{kind}/{id}`).
- **Reading modes**: each user's right to left, left to right or continuous scroll, kept per series (`PUT /api/v1/series/{id}/reading_mode`); until chosen, the direction a CBZ's or image folder's `ComicInfo.xml` states, else continuous for a PDF and left to right for the rest.
- **Library and series management** (admin only): creating, editing and deleting libraries and choosing their series (`/api/v1/libraries…`); editing series, and deleting one without books (`/api/v1/series…`); choosing the cover a series or a library shows (`…/cover`).
- **Sign-up** with email verification (`POST /api/v1/auth/register`, `/auth/verify_email`, `/auth/verify_email/resend`): the first account needs the setup code from the server's start log and becomes the admin; without email set up, an address counts as verified at once; an account left unverified for 48 hours frees its name and address for the next sign-up.
- **Login and sessions** with username or email (`POST /api/v1/auth/login`, `/auth/refresh`, `/auth/logout`): a 15-minute session token and a rotating 30-day refresh token; repeated wrong passwords are throttled.
- **Two-factor auth** with an authenticator app (`/api/v1/account/totp…`, `/auth/login/totp`, `/auth/login/recovery_code`): ten single-use recovery codes that can be renewed, and a lock on wrong codes.
- **Password reset** by emailed code (`POST /api/v1/auth/password_reset`, `/auth/password_reset/confirm`): it also confirms the email address and ends every session.
- **Account changes** (`/api/v1/account/password`, `/account/username`, `/account/email`, `/account/email/confirm`): the password (the other sessions end), the username, and the email — confirmed by a code sent to the new address, or at once without email.
- **Language setting** per user (`GET` and `PUT /api/v1/account/settings`, or chosen at sign-up): one of the languages the server offers. The code mails — verification, password reset, email change — are written in it, else in the default language, English.
- **Translation catalog** in English and German (`GET /api/v1/locales`, `GET /api/v1/locales/{code}/translations`): the apps' texts by namespace, English wherever a language lacks one, sent with an ETag so an app re-checks its copy without downloading it again. The texts ship in `translations/` and are synced into the database at every start.
- **User management** (`GET /api/v1/users`, `PATCH /api/v1/users/{id}`, admin only): every account, its role, and marking an email verified; the last admin stays an admin. Every user reads their own account and role at `GET /api/v1/account`.
- **HTTPS** through `tailscale serve` ([tailscale.md](run-deploy/tailscale.md)), with `BIND_ADDRESS` (by default `127.0.0.1`) and `PORT` choosing where the server listens.
- **Health check** (`GET /health`).

## App

- **Downloads** for macOS, Windows and Linux as unsigned preview builds on the app repo's Releases page.
- **Connecting** to a server by its address — `https://`, or `http://` to this computer — with the login kept in the system's secure storage and the session renewed by itself.
- **Account screens**: login with username or email, the two-factor code or a recovery code, sign-up with the setup code, email verification, password reset and sign-out.
- **Settings**: the language, two-factor auth on and off with a QR code, recovery codes shown once and renewable, and on phones and tablets a left- or right-handed layout.
- **Browsing** the libraries, a library's series and a series' books as grids of covers, with their counts, a user menu, and a sidebar for switching libraries.
- **Manage screen** (admins only): adding and removing book folders, creating, editing and deleting libraries and series, choosing their series and their covers.
- **Readers** for PDF (loaded in parts as it is read), flowing and fixed-layout EPUB, CBZ and image folders.
- **Reader controls** in a bar that hides while reading: right to left, left to right or continuous scroll, kept per series on the server; zoom steps; the font size of flowing EPUBs; the book's table of contents and its links inside the book.
- **Languages**: English and German from the server's catalog, kept on the device and re-checked by its ETag; English built in for a start without the server.

Next milestone: Tier 3 — read state, search, sorting and paging, and book metadata — see the [checklist](build-checklist.md).
