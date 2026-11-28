# 09 Metadata enrichment — hand checks

## Step 4a · Adult search toggle
- [ ] Stop the server and start it again with `cargo run`; the log shows `Applied migration 0012 adult_metadata` and no error
- [ ] Log in: `curl -s -X POST http://127.0.0.1:8080/api/v1/auth/login -H 'Content-Type: application/json' -d '{"username_or_email":"PASTE_USERNAME","password":"PASTE_PASSWORD","device_name":"curl","platform":"macos"}' -w '\n'` — `PASTE_USERNAME` / `PASTE_PASSWORD` are your admin account's. Keep `session_token` from the answer (not `refresh_token`). If it answers `"status":"totp_required"` instead, send `curl -s -X POST http://127.0.0.1:8080/api/v1/auth/login/totp -H 'Authorization: Bearer PASTE_CHALLENGE_TOKEN' -H 'Content-Type: application/json' -d '{"code":"PASTE_APP_CODE"}' -w '\n'` with `challenge_token` from the first answer and the six digits your authenticator app shows now (not a recovery code), and keep `session_token` from that answer
- [ ] `curl -s http://127.0.0.1:8080/api/v1/libraries -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -w '\n'` lists every library with `"adult_metadata":false`
- [ ] `curl -s -X PATCH http://127.0.0.1:8080/api/v1/libraries/PASTE_ID -H 'Authorization: Bearer PASTE_SESSION_TOKEN' -H 'Content-Type: application/json' -d '{"adult_metadata":true}' -w '\n'` answers that library with `"adult_metadata":true` and its name, type and description as before — `PASTE_ID` is the `id` of any library in the list before
- [ ] The same command with `-d '{"adult_metadata":"yes"}'` answers `{"error":"invalid_request"}` — only `true` or `false`
- [ ] In the app, Manage → Libraries → that library: change its description and save. The list command above still shows `"adult_metadata":true` for it — this app version doesn't know the setting, and saving its form leaves it alone
- [ ] The PATCH command with `-d '{"adult_metadata":false}'` answers `"adult_metadata":false` — back as it was; set the description back in the app if you like
