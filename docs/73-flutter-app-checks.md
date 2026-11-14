# Flutter app — checks

First: stop the server and start it again ([`run`](run-deploy/local.md#commands)). You need one account without two-factor; if yours all have it, make one — `curl -X POST http://localhost:8080/api/v1/auth/register -H 'Content-Type: application/json' -d '{"email":"PASTE_EMAIL","username":"PASTE_USERNAME","password":"PASTE_PASSWORD","password_repeat":"PASTE_PASSWORD","device_name":"terminal","platform":"macos"}' -w '\n'` (any new address, name and password of your choice; only the very first account needs a setup code).

## macOS
- [x] Step 5 · `cd ~/bookserver-front-end && flutter run -d macos` opens on "Connect to your book server"
- [x] Step 5 · `not a server`, then Connect: "This doesn't look like a server address."
- [x] Step 5 · `http://localhost:9999` (nothing listens there): "Can't reach the server. Check the address and your connection."
- [x] Step 5 · `https://example.com`: "There's no book server at this address."
- [x] Step 5 · `http://localhost:8080`: the "Sign in" screen, with `localhost:8080` under its title
- [x] Step 5 · Your username and a wrong password: "Wrong username, email address or password."
- [x] Step 5 · The right password: "Libraries"
- [x] Step 5 · Quit with `q` in the app's terminal and start it again with the same `flutter run` command: it opens on "Libraries". Say whether macOS asked for keychain access
- [x] Step 5 · `sqlite3 ~/bookserver_backend/data/databases/app_data.sqlite "SELECT device_id, device_name, platform FROM sessions WHERE device_name = 'macOS'"` shows one row ending in `|macOS|macos`; note its device id, the part before the first `|`
- [x] Step 5 · The sign-out button at the top right leads to "Sign in", and the same `sqlite3` command shows no row
- [x] Step 5 · Sign in again: the `sqlite3` command shows the device id you noted
- [x] Step 5 · Sign out, then sign in with an account that has two-factor: "This account uses two-factor authentication, which this version of the app can't do yet."
- [x] Step 5 · "Change server" leads to "Connect to your book server"
- [x] Step 5 · Quit, start `cd ~/bookserver-front-end && flutter run -d macos --release`, enter `http://localhost:8080`: "Use an address that starts with https://."
- [x] Step 5 · Every text on the address and sign-in screens is one of the texts in `~/bookserver_backend/translations/en/frontend/server.json` and `login.json` — none comes from anywhere else

## At the end of the plan: iOS simulator and Android emulator
- [ ] Step 5 · `open -a Simulator`, then `cd ~/bookserver-front-end && flutter run -d PASTE_DEVICE_ID` (the iPhone's id from `flutter devices`): `http://localhost:8080` and signing in reach "Libraries"; quit with `q`, run the same command again, and it opens on "Libraries"
- [ ] Step 5 · `flutter emulators --launch PASTE_EMULATOR_ID` (an id from `flutter emulators`), then `cd ~/bookserver-front-end && flutter run -d PASTE_DEVICE_ID` (the emulator's id from `flutter devices`, like `emulator-5554`): `http://10.0.2.2:8080` and signing in reach "Libraries"; quit with `q`, run the same command again, and it opens on "Libraries"
