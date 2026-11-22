# File watching

State: completed

New and changed files in the scannable directories are picked up on their own: filesystem
events, debounced, trigger a re-scan of just the folders that changed. It needs
[incremental scanning](34-directory-scanning.md) — otherwise every event costs a full pass —
and the [directories table](35-scannable-directories.md) to know what to watch.

## Build
- [x] 4 h · Watch the scannable directories with `notify` — it calls back on its own thread, so forward the events into a `tokio::sync::mpsc` channel; debounce in a plain struct that takes `now`, so tests never sleep: a folder is due after 10 s without events, 5 min at the latest [BE]
- [x] 3 h · Turn the due folders into one request for [34](34-directory-scanning.md)'s scan job: a change rescans the folder holding it — so a folder created or moved in is read whole — and "rescan needed" the whole directory; access and metadata-only events, dotfiles, `.part` files and `@eaDir` are ignored, and a folder whose files changed within the last 10 s waits again. Removed files go through [53](53-file-reconciliation.md) [BE]
- [x] 2 h · Start and stop watching a directory when it is added to or removed from the table, and report in `GET /directories` whether each one is watched [BE]
- [x] 0.5 h · Check by hand, in a folder on the Mac's own disk (`~/Documents`): a supported file dropped into a watched directory shows up in the catalog within the debounce window, without a scan request
