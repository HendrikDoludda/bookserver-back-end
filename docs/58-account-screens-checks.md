# 58 Account screens — checks

Before starting: stop the server and start it again with the `run` command from [local.md](run-deploy/local.md) — without the mail settings, so the test account below is ready at once. Then stop the app and start it again with `flutter run -d macos`; the new build asks for the keychain: enter your Mac password and click **Always Allow** (Deny signs you out and proves nothing).

These checks use a separate test account, `tester`, so your own stays untouched. Keep it and its entry in your authenticator app: the 2FA settings checks later use them again.

- [ ] Make the test account: `curl -s -X POST http://localhost:8080/api/v1/auth/register -H 'Content-Type: application/json' -d '{"email":"tester@example.com","username":"tester","password":"tester password 123","password_repeat":"tester password 123","device_name":"terminal","platform":"macos"}' -w '\n'` → an answer starting `{"status":"ok","session_token":…`. Its `session_token` is what the next two commands need (not `refresh_token`); it lasts 15 minutes.
- [ ] Start 2FA for it: `curl -s -X POST http://localhost:8080/api/v1/account/totp -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` — `PASTE_SESSION_TOKEN`: `session_token` from the answer above. In your authenticator app, add an account by setup key (not by QR): name `tester`, key = `secret_base32` from this answer, time-based.
- [ ] Turn it on: `curl -s -X POST http://localhost:8080/api/v1/account/totp/confirm -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"code":"PASTE_APP_CODE","password":"tester password 123"}' -w '\n'` — `PASTE_APP_CODE`: the 6 digits your authenticator app shows for `tester` right now. → `{"recovery_codes":[…]}`; copy two of them somewhere.
- [ ] In the app, sign out of your account and sign in as `tester` / `tester password 123` → the "Two-factor authentication" step. Enter the app's code → the libraries.
- [ ] Sign out, sign in as `tester` again; on the code step, letters can't be typed into the code field. Enter `000000` → "This code is wrong or has expired."
- [ ] Click "Use a recovery code instead", enter one of the saved recovery codes → the libraries.
- [ ] Sign out, sign in as `tester`, and on the code step click Back (top left) → the login screen. Then sign in to your own account again.
