# Left- and right-handed controls

State: completed

A left- or right-handed choice, kept on each phone or tablet, that moves the on-screen controls
to the side of the hand holding it: the reader bar's buttons ([83](83-app-layout.md)) and, once
the app has any, its floating action buttons. Page order and tap zones follow the
[reading direction](80-reading-direction.md) instead.

## Build
- [x] 0.5 h · Texts for the setting — its label, "Left-handed" and "Right-handed" — in a `settings` namespace in `translations/en/frontend/` and `translations/de/frontend/` [BE]
- [x] 1 h · Keep the choice on the device — in `preferences.json` in the app's support folder, which [81](81-reader-view-settings.md)'s font size shares — right-handed until one is made, outside the session record so it survives logging out and changing servers [FE]
- [x] 1 h · Provide the choice app-wide: any widget reads the thumb side and rebuilds when it changes [FE]
- [x] 1 h · Left/right-handed switch on the settings screen ([58](58-account-screens.md)), on phones and tablets only; macOS, Windows and Linux keep the right-handed default [FE]
- [x] 1 h · The reader bar's buttons gather at its thumb-side end, placed by physical side, not by text direction [FE]
- [x] 0.25 h · Rule in the frontend's `docs/ARCHITECTURE.md`: a floating action button sits in the bottom corner on the thumb side, placed by physical side, not by text direction [FE]
- [x] 0.5 h · Check by hand on a phone: left-handed puts the reader buttons on the left, and the choice survives an app restart and a log-out
