# Release pipeline

State: blocked

**Blocked:** needs the `.dmg` / `.exe` packaging from
[Packaging & deployment](52-packaging-and-deployment.md), and the release workflow from
[Downloadable builds](85-downloadable-builds.md).

Every version tag produces signed macOS and Windows packages, published with release notes
where installs — and the [updater](25-desktop-auto-update.md) — can fetch them. Nothing exists
yet: no CI, no published builds, and the version in `Cargo.toml` (`0.1.0`) is read by nothing
at runtime.

## Build
- [ ] ? h · 85's release workflow also builds 52's `.dmg` and `.exe` for each version tag and publishes them with release notes on this repo's Releases [BE]
- [ ] ? h · macOS signing: Apple Developer ID plus notarization (the Apple Developer Program, about $99 a year) — without it Gatekeeper blocks the app for anyone but you [BE]
- [ ] ? h · Windows signing with a code-signing certificate — unsigned installs trigger SmartScreen warnings [BE]
