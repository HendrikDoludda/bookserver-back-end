# File reconciliation

State: not started

A scan only adds files that aren't in the database yet, so a book whose file was moved or
deleted keeps its row forever. Reconciliation, part of every scan, handles the rows the walk no
longer finds: a moved file keeps its row, a deleted one loses it — unless its whole directory is
unavailable, so an unplugged drive empties nothing.

## Build
- [ ] 2 h · A moved file keeps its row: before a new file is inserted, it takes over a row with its `file_hash` that this scan's walk didn't see, or, outside the scan's scope, whose file is gone [BE]
- [ ] 2 h · After a scan's inserts and moves, delete the rows in its scope that its walk didn't see, and the series left without books — never for a directory whose root is missing or unreadable or holds no supported file [BE]
- [ ] 1 h · A full scan deletes the rows that belong to no directory [BE]
- [ ] 2 h · Delete the files in the cover folder that no book, series or library row names, except the `example-cover.png` fallback, inside the scan job — today a cover outlives its book [BE]
