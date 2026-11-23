# Downloadable builds — checks

Push first: the workflow runs on GitHub's copy of the repo. The first run takes about half an hour.

- [ ] The manual run builds everything: on the repo's GitHub page, Actions → Release → Run workflow, branch `master`. All six jobs turn green — `test` on Ubuntu, macOS and Windows, `package` for macos-universal, windows-x64 and linux-x64 — and `release` is skipped. For a red job, open it, and paste the last lines of its failing step into the chat.
- [x] The finished run's page lists three artifacts at the bottom: `package-macos-universal`, `package-windows-x64` and `package-linux-x64`.
