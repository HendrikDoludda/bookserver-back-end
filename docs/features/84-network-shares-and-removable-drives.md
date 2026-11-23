# Network shares and removable drives

State: in progress

Books on a NAS share or an external drive: changes the system doesn't report, a share that stops
answering, a drive unplugged and plugged back in.

## Build
- [ ] ? h · A per-folder "watch" switch, and periodic incremental scans of the folders that aren't watched: a network share (SMB, NFS) reports no change made from another machine, so books copied onto a NAS from elsewhere show up only after a scan request [BE]
- [x] 4 h · A scan stuck in a file-system call — an NFS hard mount or an SMB share whose server is gone blocks for ever — is given up on after 2 minutes without a sign of life and recorded as `stalled`; it runs on a thread of its own, writes nothing once given up on, and the worker runs the next scan [BE]
- [x] 3 h · Each folder has a watcher of its own, and every call into it runs under a 30 s limit, so one dead share neither freezes the other folders' watching nor holds back the server's start [BE]
- [x] 3 h · On Linux (inotify) a watched folder that disappears — an unmounted drive — loses its watch for good, while `watching` still says true: check every minute whether each watched folder is still there and the same, mark it unwatched while it is gone, and watch it again and scan it once it is back [BE]
- [ ] 1.5 h · On Windows, notice a watched folder that was replaced or plugged back in: `identity()` in `folder_watch.rs` is `None` there, so the minute's check never sees a change — read the folder's volume serial number and file index (`GetFileInformationByHandle` through `windows-sys`, for Windows only), and drop the `#[cfg(unix)]` from the three folder-replacement tests [BE]
- [ ] 0.5 h · Check by hand on Windows: a watched book folder on a USB drive, unplugged and plugged back in, is watched again, and a book copied onto it afterwards shows up without a scan request
- [x] 0.5 h · A cover is written to a temporary name and then renamed, and an image folder's page 1 is read into memory first — a write cut short leaves a broken cover that every later scan reuses [BE]
- [ ] ? h · Refuse file requests into a folder that stopped answering (503) instead of starting more hung threads: reading a page, streaming a file and adding a folder each keep a blocking-pool thread for good on a dead share, and once all 512 are taken every database call waits [BE]
- [ ] 0.5 h · Check by hand with a folder shared over SMB from your personal Mac: start a scan of it, cut the share off, and within about 2 minutes `scan-status` shows the scan `failed` with a `stalled` failure, and a new `scan` is accepted
