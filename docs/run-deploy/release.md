# Release

## Commands
| Key | Command |
|---|---|
| `lockfile` | `cargo check` |
| `tag` | `git tag -a vPASTE_VERSION -m "bookserver PASTE_VERSION"` |
| `push-tag` | `git push origin vPASTE_VERSION` |
| `delete-tag` | `git tag -d vPASTE_VERSION && git push origin --delete refs/tags/vPASTE_VERSION` |

## Deploy
1. Raise `version` in `Cargo.toml`, bring `Cargo.lock` in line — [`lockfile`](#commands) — and commit both: the workflow builds with `--locked` and stops on a lock file that disagrees. The first release keeps `0.1.0`.
2. Tag that commit — [`tag`](#commands) — and push the tag — [`push-tag`](#commands). `PASTE_VERSION` is `Cargo.toml`'s `version`, such as `0.1.0`.
3. Watch the run on the repo's GitHub page, Actions → Release: three test jobs, three package jobs, then `release`, which refuses a tag that isn't `v` and `Cargo.toml`'s version.
4. Under Releases, the draft named after the tag holds `bookserver-server-<version>-macos-universal.zip`, `-windows-x64.zip` and `-linux-x64.tar.gz`. Try them before anyone else can: 85's hand checks.
5. Publish it: the draft's Edit → Publish release.
6. A red run or a bad draft: delete the draft on the Releases page and the tag — [`delete-tag`](#commands), which leaves the commit alone — then fix, commit, and start again at step 2.
