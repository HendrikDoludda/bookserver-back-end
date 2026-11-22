# Downloadable builds

State: in progress

A zip per platform — macOS (Intel and Apple Silicon in one), Windows x64, Linux x64 — holding
the server, PDFium, a README and the licence notices, built by GitHub Actions for each version
tag and published on this repo's Releases. The server keeps everything it writes in one data
folder, so it runs from wherever it was unpacked. Installers, Docker and HTTPS inside the
server stay in [52](52-packaging-and-deployment.md).

## Build
- [x] 2 h · One data folder holds the database (`app_data.sqlite`), the covers (`covers/`) and `secrets.toml`: `DATA_DIR`, else the OS's local data folder for the identifier `com.hendrikdoludda.bookserver` (`ProjectDirs::from("com", "hendrikdoludda", "bookserver")`); a `DATA_DIR` that isn't a folder stops the start, naming it [BE]
- [x] 0.5 h · Log the version (`Cargo.toml`'s) and the data folder at start [BE]
- [x] 1 h · The fallback cover `example-cover.png` is compiled into the binary and written into the cover folder when it is missing [BE]
- [x] 1 h · Bind PDFium from the binary's folder first, then the working directory, then the system's library path [BE]
- [ ] 0.5 h · `BIND_ADDRESS` defaults to `127.0.0.1` [BE]
- [ ] 0.5 h · SQLite compiled into the binary (`rusqlite` feature `bundled`) [BE]
- [ ] 2 h · Licence check and notices: `cargo about` accepts only the licences listed in `about.toml` and renders `THIRD_PARTY_LICENSES.html` [BE]
- [ ] 1 h · `packaging/README.txt`, shipped in every download: starting the server on each OS, the setup code, the data folder and the settings, `tailscale serve`, and the terms — you may download and run this build to try it out; all other rights reserved [BE]
- [ ] 2 h · `scripts/package.sh <platform>`: the release binary (macOS: both CPUs joined by `lipo`), PDFium `chromium/7543` (`pdfium-mac-univ.tgz`, `pdfium-win-x64.tgz`, `pdfium-linux-x64.tgz`) checked against a pinned SHA-256 — its macOS library ships unsigned, so it is ad-hoc signed —, the README, the licence notices and PDFium's own (`LICENSE` and `licenses/` in each archive), zipped [BE]
- [ ] 3 h · `.github/workflows/release.yml`: the tests on Ubuntu, macOS and Windows, a package per platform, and for a `v*` tag that matches `Cargo.toml`'s version a draft GitHub Release with the zips [BE]
- [ ] 1 h · Release builds of the app accept `http://` to `localhost`, `127.0.0.1` and `::1`; every other address still needs `https://` [FE]
- [ ] 0.5 h · `run-deploy/release.md`, and a Download section in the README
- [ ] 0.5 h · Check by hand on your other Mac: the macOS download from the draft release starts after clearing its quarantine, logs the version, the data folder, PDFium and the setup code, gives a PDF a cover, and the phone app reaches it through `tailscale serve`
- [ ] 0.5 h · Check by hand on your Windows PC: the Windows download starts without a firewall prompt, keeps its data in `%LOCALAPPDATA%\hendrikdoludda\bookserver\data` and gives a PDF a cover
- [ ] ? h · Check by hand: the Linux download on a Linux desktop
- [ ] ? h · Check by hand: the Windows download on a Windows without Visual Studio (Windows Sandbox, on Windows Pro)

## Missing information
- [ ] Do the tests pass on Windows and Linux? They have only ever run on macOS — answered by: the release workflow's first run
- [ ] Does `aws-lc-sys` (the TLS behind `mail-send`) need NASM on the Windows runner? — answered by: the release workflow's first run
- [ ] Does `pdfium.dll` need the Visual C++ runtime? — answered by: the check on a Windows without Visual Studio
