# Packaging & deployment

State: not started

Ship the server as a Docker image, a macOS `.dmg` and a Windows `.exe`, keeping
[85](85-downloadable-builds.md)'s data folder outside the app so updates and containers keep it.
[Desktop auto-update](25-desktop-auto-update.md) builds on the installers.

## Build
- [ ] ? h · Graceful shutdown: `axum::serve` gets `.with_graceful_shutdown(signal)`, fired by `SIGTERM` and Ctrl-C, so in-flight requests finish and pooled SQLite connections close — `docker stop` sends `SIGTERM` and kills the process 10 seconds later; the tray's "Quit" ([77](77-tray-app.md)) and the updater's restart ([25](25-desktop-auto-update.md)) fire it too [BE]
- [ ] 3 h · Serve HTTPS without `tailscale serve` ([32](32-https-tls.md)) through `axum-server` 0.9 (feature `tls-rustls`): `RustlsConfig::from_pem_file(cert, key)` and `bind_rustls`, with a certificate per the LAN-HTTPS question [BE]
- [ ] 1 h · Settings for the certificate and key paths [BE]
- [ ] 2 h · Pick up a renewed certificate without a restart (`RustlsConfig::reload_from_pem_file`), triggered per the reload decision [BE]
- [ ] ? h · The desktop installs show the setup code the first sign-up needs ([27](27-registration-and-email-verification.md)) — an app started from Finder or the Start menu has no visible log [BE]
- [ ] ? h · Docker image with a persistent volume for the data folder (`DATA_DIR`, [85](85-downloadable-builds.md)), and `BIND_ADDRESS=0.0.0.0` — a published port reaches the container's network interface, not its `127.0.0.1` [BE]
- [ ] ? h · Publish the Docker image for each version, per the registry decision, with 85's terms note [BE]
- [ ] ? h · macOS `.dmg`, unsigned for now — macOS refuses the first launch until it is allowed in System Settings → Privacy & Security; signing comes with [76](76-release-pipeline.md) [BE]
- [ ] ? h · Windows `.exe`, unsigned for now — SmartScreen warns before the first launch; signing comes with [76](76-release-pipeline.md) [BE]
- [ ] ? h · Put 85's licence notices (`THIRD_PARTY_LICENSES.html` and PDFium's) into the Docker image, the `.dmg` and the `.exe` [BE]

## Open decisions
- [ ] Where is the Docker image published? — blocks: publishing the image. Suggested: Docker Hub — a public image needs no public source repo.
- [ ] What triggers the certificate reload — a timer, a signal, or watching the files? — blocks: picking up a renewed certificate. Suggested: a daily timer — certificates last weeks, so polling is plenty, needs no extra crate, and works on Windows, which has no signals.

## Missing information
- [ ] How does a LAN device without Tailscale get HTTPS — which name does it resolve, and who issues the certificate for that name (own domain with local DNS, a local CA)? A `*.ts.net` certificate only validates where MagicDNS resolves the name — answered by: you, before the in-server TLS tasks
