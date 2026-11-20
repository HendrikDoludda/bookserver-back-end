# Flutter app

State: in progress

The main client: one Flutter app for phone and desktop, built alongside the server so each
feature can be tested in it. The desktop version is a client only — the server's own desktop
install is [52](52-packaging-and-deployment.md).

## Build
- [x] 1 h · Texts for the server-address, login and home screens and for failed connections, in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [x] 2 h · Replace the template app: the folder layout, and design tokens for the first screens [FE]
- [x] 2 h · Server address screen: an `https://` origin — `http://` in debug builds only — accepted when `GET /health` answers `pong` [FE]
- [x] 1 h · Network access: the INTERNET permission in Android's main manifest, the outgoing-network entitlement on macOS [FE]
- [x] 2 h · API client: Bearer tokens and JSON bodies to the server's origin, redirects never followed; `{"error": key}` answers become typed errors, a failed connection or a timeout a connection error [FE]
- [x] 2 h · Keep the server's origin, the device id and the session tokens as one record in the platform's secure storage — macOS's legacy keychain, no Android backup — and open the app on the screen the record calls for; an unreadable record, or one whose server this build may not use (`http://` in a release build), counts as none [FE]
- [x] 1 h · Refresh the session token when it has less than a minute left, and once after a 401; a refused refresh ends the session, a network failure doesn't [FE]
- [x] 1 h · Refresh runs one at a time: parallel requests wait for that single refresh instead of each starting their own [FE]
- [x] 1 h · Placeholder home: the libraries' names from `GET /api/v1/libraries`, with a reload action [FE]
- [x] 2 h · Android build — a universal release APK to sideload, signed with your own release key from `android/key.properties` (outside git); without it a release build stops [FE]
- [ ] 3 h · iOS release build on your own iPhone and iPad, signed with your free personal Apple team from your personal Mac; the signature lasts 7 days, then the app installs again from that Mac and keeps its session [FE]
- [ ] 0.5 h · Check by hand on the iPhone, installed from your personal Mac: the release build starts, signs in, opens a PDF, an EPUB and a CBZ, and is still signed in after it was closed; "Open in authenticator app" opens an authenticator app if one is on the phone; the work Mac's iOS simulator still runs the app
- [ ] 0.5 h · Check by hand on the iPad: the release build installs (iPadOS 15 or newer), starts, signs in and opens a PDF, an EPUB and a CBZ
- [x] 1 h · macOS release build, zipped to copy onto another Mac [FE]
- [ ] 2 h · Windows release build — the `windows/` folder, built on a Windows PC with Visual Studio's C++ workload [FE]
- [x] 2 h · Linux release build — the `linux/` folder, built in Docker on the Mac [FE]
- [ ] 0.5 h · Check by hand on the Windows PC: the release build starts, signs in, opens a PDF, an EPUB and a CBZ, and is still signed in after a restart
- [ ] 0.5 h · Check by hand on a Linux desktop: the release build starts, signs in and opens a PDF, an EPUB and a CBZ
- [ ] 1 h · Sign macOS debug builds with your free personal Apple team on your personal Mac — a per-Mac setting outside git, since the work Mac holds no personal certificate — so a rebuild reads the stored session without a keychain prompt [FE]
- [ ] ? h · Sign the macOS release build with an Apple Developer Program identity (Developer ID or the Mac App Store), so an update keeps keychain access to the stored session [FE]

## Open decisions
- [ ] When do the apps go into the App Store and Play Store? — blocks: signing the macOS release build. Suggested: once someone other than you uses the server; the Apple Developer Program fee also covers the macOS signing in [76](76-release-pipeline.md).
- [ ] A web frontend as well, eventually? — blocks: nothing yet. Suggested: decide once the app covers reading — under the API-first rule it would be one more client of the same API.
