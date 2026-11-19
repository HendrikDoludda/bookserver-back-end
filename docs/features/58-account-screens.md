# Account screens

State: in progress

The app's screens for the account flows the server offers — login, two-factor auth and recovery
codes, sign-up and email verification, password reset, and the language setting — built on the
[app skeleton](73-flutter-app.md).

## Build
- [x] 3 h · Login screen: username or email plus password [FE]
- [x] 1 h · Log-out action [FE]
- [x] 0.5 h · Texts for the code-entry step in a `login_code` namespace, in `translations/en/frontend/` and `translations/de/frontend/`; `login.totp_unavailable` deleted [BE]
- [x] 2 h · Code-entry step after the password, when login answers `totp_required`; it replaces the `login.totp_unavailable` notice [FE]
- [x] 1 h · "Use a recovery code" option on the code-entry step [FE]
- [x] 0.5 h · Texts for sign-up and verification in `sign_up` and `verify_email` namespaces, and the login screen's link to sign-up [BE]
- [x] 3 h · Sign-up screen [FE]
- [x] 2 h · Verification-code screen, with "send a new code"; reached from sign-up and from a login answering `email_not_verified`, with the password the user just typed [FE]
- [x] 0.5 h · Texts for the password reset in a `password_reset` namespace, and the login screen's link to it [BE]
- [x] 2 h · "Forgot password" form [FE]
- [x] 2 h · Code and new-password screen [FE]
- [x] 0.5 h · Texts for the settings screen and its language picker in a `settings` namespace, and the settings button's tooltip [BE]
- [x] 0.25 h · A temporary settings button next to Sign out in the library screen's top bar, until [83](83-app-layout.md)'s user menu [FE]
- [x] 2 h · Settings screen with a language picker; its "Device language" entry is written in the language choosing it shows [FE]
- [x] 1 h · Each language's icon in the picker's list, from `GET /api/v1/locales` [FE]
- [ ] 0.5 h · Texts for the 2FA settings in a `two_factor` namespace [BE]
- [ ] 4 h · 2FA settings: enable (QR code + confirm) and disable [FE]
- [ ] 1 h · Show the recovery codes once, right after 2FA is switched on [FE]
- [ ] 1 h · Renew the recovery codes from the 2FA settings, shown once like the first ones [FE]
