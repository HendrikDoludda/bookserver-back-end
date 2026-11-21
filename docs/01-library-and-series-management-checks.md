# Checks — 01 Library & series management

In the app's debug build on this Mac. Don't save a new name for one of your own series yet: until step 10, the scanner finds a series by its stored name.

- [ ] Stop the server and start it again with [`run`](run-deploy/local.md#commands) — the new texts are compiled into this build.
- [ ] Start the app from the `bookserver-front-end` folder with [`run-macos`](../../bookserver-front-end/docs/run-deploy/local.md#commands). A new build asks for the keychain, where the session is kept: enter your Mac password and click Always Allow — Deny proves nothing.
- [ ] Signed in as the admin, the account menu at the top right shows Settings, **Manage** and Sign out; Manage opens a screen with Libraries and Series.
- [ ] Libraries lists your libraries with their series counts. The **+** at the top creates "Favourites": type Comics, a description, two of your series ticked — the ones in no library come first. Save closes the form, and the list shows "Favourites" with 2 series.
- [ ] Back on the home screen, after Reload, "Favourites" holds those two series.
- [ ] Open "Favourites" in Manage → Libraries: rename it, empty its description, untick one series, Save — the list shows the new name with 1 series.
- [ ] Manage → Series lists every series with its book count. Open one with books, press Delete and confirm — the form says the series still has books, and nothing is deleted.
- [ ] Open "Favourites" again, press Delete: Cancel keeps it; Delete removes it from the list, and its series are still in their own libraries.
- [ ] In Settings choose Deutsch: Manage and its screens are in German ("Verwalten", "Bibliotheken", "Serien"). Switch back afterwards.
- [ ] Signed in as a member — sign one up on the login screen if you have none — the account menu shows no Manage.
