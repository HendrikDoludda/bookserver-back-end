bookserver — server
===================

A home server for books, manga and comics: it scans your folders, extracts the
covers and serves everything to the bookserver app.
Source: https://github.com/HendrikDoludda/bookserver-back-end

This folder holds:
  bookserver (bookserver.exe)   the server
  libpdfium / pdfium.dll        reads PDFs for their covers; keep it beside the server
  LICENSE                       this project's licence
  THIRD_PARTY_LICENSES.html     the licences of the code it is built from
  pdfium/  notices/             PDFium's licences, and notices that code asks to ship


START IT
--------
The server runs in a terminal window and writes its log there. Ctrl-C stops it.

macOS
  The download isn't signed by Apple, so macOS blocks it until its download
  flag is cleared, once. In Terminal (drag the folder onto the window to type
  its path):
    xattr -dr com.apple.quarantine <this folder>
    <this folder>/bookserver

Windows
  Right-click inside this folder -> "Open in Terminal" (Windows 10: Shift +
  right-click -> "Open PowerShell window here"), then:
    .\bookserver.exe
  If SmartScreen warns about an unknown app: "More info" -> "Run anyway".

Linux
  ./bookserver

Check that it answers: http://localhost:8080/health in a browser says "pong".


THE FIRST ACCOUNT
-----------------
While no account exists, the log shows:
  No account exists yet: the first sign-up needs the setup code <code>
The first sign-up must send that code, and becomes the admin. Sign up in the
bookserver app, or from a second terminal (the password needs 12 characters
or more):

macOS / Linux:
  curl -X POST http://localhost:8080/api/v1/auth/register -H 'Content-Type: application/json' -d '{"email":"EMAIL","username":"USERNAME","password":"PASSWORD","password_repeat":"PASSWORD","device_name":"terminal","platform":"terminal","setup_code":"SETUP_CODE"}'

Windows (PowerShell):
  Invoke-RestMethod -Method Post -Uri http://localhost:8080/api/v1/auth/register -ContentType 'application/json' -Body '{"email":"EMAIL","username":"USERNAME","password":"PASSWORD","password_repeat":"PASSWORD","device_name":"terminal","platform":"terminal","setup_code":"SETUP_CODE"}'


YOUR DATA
---------
Everything the server writes (its database, the covers and secrets.toml) lies
in one data folder, which the log names at start:
  macOS    ~/Library/Application Support/com.hendrikdoludda.bookserver/
  Windows  %LOCALAPPDATA%\hendrikdoludda\bookserver\data\
  Linux    ~/.local/share/bookserver/
Deleting it resets the server: every account and the whole catalogue are gone.


SETTINGS
--------
Environment variables, set when starting it:
  macOS / Linux:  PORT=9000 ./bookserver
  PowerShell:     $env:PORT = "9000"; .\bookserver.exe

  DATA_DIR       another data folder; made if missing
  PORT           the port to listen on; 8080 by default
  BIND_ADDRESS   127.0.0.1 by default: only this computer reaches the server.
                 0.0.0.0 opens it to your network, unencrypted.
  SMTP_HOST      a mail server for verification and password-reset codes;
                 email is off without it. With it, SMTP_PORT, SMTP_FROM and
                 SMTP_FROM_NAME are required; SMTP_USERNAME and SMTP_PASSWORD
                 are optional but go together; SMTP_TLS is starttls (default),
                 implicit or none.

A setting the server can't use stops it at start.


PHONES AND OTHER DEVICES
------------------------
Tailscale (https://tailscale.com) gives them an HTTPS address for the server:
  1. Install Tailscale on this computer and the phone, and sign in on both.
  2. In the Tailscale admin console, under DNS, turn on MagicDNS and HTTPS
     Certificates.
  3. With the server running:  tailscale serve --bg 8080
     It prints the https://<computer>.<tailnet>.ts.net address the app
     connects to.
To stop it:  tailscale serve --https=443 off


LICENCE
-------
bookserver is MIT-licensed: see LICENSE. The code it is built from keeps its
own licences, in THIRD_PARTY_LICENSES.html, pdfium/ and notices/.
