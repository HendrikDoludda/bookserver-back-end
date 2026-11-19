# 58 Account screens — checks

Before starting: start Mailpit in a second terminal, then stop the server and start it again with the `run-mail` command — both in [local.md](run-deploy/local.md). Then stop the app and start it again with `flutter run -d macos`; the new build asks for the keychain: enter your Mac password and click **Always Allow** (Deny signs you out and proves nothing).

These checks reset the password of `newbie` from the sign-up checks.

- [ ] In the app, sign out; on the login screen click "Forgot your password?" → "Reset your password". "Send code" stays grey until an address is typed.
- [ ] Enter `newbie@example.com`, "Send code" → "Set a new password". In Mailpit (http://localhost:8025) a mail "Your password reset code" has arrived.
- [ ] Enter its code and a new password twice (12 characters or more), "Change password" → back on the login screen, with "Your password is changed. Sign in with the new one." and `newbie@example.com` already in the name field.
- [ ] Sign in with the old password → "Wrong username, email address or password."; with the new one → the libraries.
- [ ] Sign out; "Forgot your password?" with an address no account uses → "Set a new password" all the same, and no mail in Mailpit.
- [ ] Stop the server and start it again with the plain `run` command (no mail); "Forgot your password?", any address, "Send code" → "This server can't send email yet. Ask its admin to set it up." Then sign in to your own account again.
