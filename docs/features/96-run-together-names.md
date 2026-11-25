# Run-together file names

State: completed

Split file and folder names written without separators — `HellboyBPRDVolume10Title`,
`hellboybprdvolume10title` — so the scanner finds their title and volume and groups their volumes
into one series. A name with a space, `_` or `-` parses as today, except that a marker followed by
a dot and a space (`Vol. 1`) counts.

## Build
- [x] 2 h · In `parse_name`, a name with no white space, `_` or `-` once tags are removed gets a space where a lowercase letter meets a capital, before the last capital of a run followed by a lowercase letter, and between a cased letter and a digit: `HellboyBPRDVolume10Title` → title `Hellboy BPRD`, volume 10 [BE]
- [x] 1.5 h · In those names, a space around `volume`, `vol` or `chapter`, in any case, when digits follow: `hellboybprdvolume10title` → title `hellboybprd`, volume 10. `v`, `c`, `p` and `book` stay unsplit — they occur inside ordinary words (`bleach12`, `notebook2`) [BE]
- [x] 0.5 h · A volume, chapter or page marker of two letters or more followed by a dot and a space marks its number, as one followed by a dot does: `Hellboy Vol. 1 Seed of Destruction` → title `Hellboy`, volume 1; `One Piece Ch. 5` → chapter 5. A lone `v`, `c` or `p` doesn't: `London c. 1850` has no chapter [BE]
- [x] 0.5 h · Check by hand: a folder of `HellboyBPRDVolume10Title.cbz` and `HellboyBPRDVolume11Title.cbz`, and one of the same two names in lowercase, each show in the app as one series with volumes 10 and 11; `Hellboy Vol. 1 Seed of Destruction.cbz` shows as volume 1 of Hellboy
