//! What a scan answers: the report of one run, and the status of the scan job.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::Serialize;

/// What a scan did, as an admin gets it.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct ScanReport {
    /// Books this scan stored.
    pub added: usize,
    /// Stored books whose file changed — or that got a cover at last — read again.
    pub updated: usize,
    /// Books found stored already, left as they are.
    pub unchanged: usize,
    /// Stored books found at another path — renamed or moved — that kept their row.
    pub moved: usize,
    /// Stored books whose files are gone, removed from the catalog.
    pub removed: usize,
    /// What couldn't be read or stored: per directory, the folders the walk couldn't read, then
    /// the books.
    pub failures: Vec<ScanFailure>,
}

/// A folder or a book a scan couldn't read or store.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanFailure {
    pub directory_id: i64,
    /// Relative to the directory; empty for the directory itself.
    pub relative_path: String,
    pub error_key: ScanProblem,
}

impl ScanFailure {
    /// The failure `error_key` at `path`, inside the directory `directory_id` at `root`.
    pub fn at(directory_id: i64, root: &Path, path: &Path, error_key: ScanProblem) -> ScanFailure {
        ScanFailure {
            directory_id,
            relative_path: path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
            error_key,
        }
    }
}

/// Why a scan skipped a whole directory, a folder in it, or a book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanProblem {
    /// The directory's folder is gone, as on an unplugged drive.
    DirectoryMissing,
    /// The directory's folder can't be read — macOS privacy settings keep some folders closed.
    DirectoryUnreadable,
    /// A folder inside the directory can't be read; the rest of it is scanned.
    FolderUnreadable,
    /// The book's file can't be opened or read to its end.
    FileUnreadable,
    /// The database didn't store the book; nothing of it was kept.
    StoreFailed,
    /// A library reading the book panicked.
    Panicked,
    /// The scan showed no sign of life for 2 minutes — a share that stopped answering — and was
    /// given up on; the path is where it got stuck.
    Stalled,
}

/// Where a scan reads: every added directory, or these folders — a directory's own, or one inside
/// it — each with everything in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanScope {
    All,
    Folders(BTreeSet<PathBuf>),
}

impl ScanScope {
    /// The folder at `path` alone.
    pub fn folder(path: impl Into<PathBuf>) -> ScanScope {
        ScanScope::Folders(BTreeSet::from([path.into()]))
    }

    /// One scope that reads what both read: a folder inside another adds nothing.
    pub fn merge(self, other: ScanScope) -> ScanScope {
        match (self, other) {
            (ScanScope::Folders(mut folders), ScanScope::Folders(more)) => {
                folders.extend(more);
                let all = folders.clone();
                folders.retain(|folder| {
                    !all.iter()
                        .any(|other| other != folder && folder.starts_with(other))
                });
                ScanScope::Folders(folders)
            }
            _ => ScanScope::All,
        }
    }
}

/// What asked for a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanTrigger {
    /// An admin, through `POST /scan` or `POST /directories/{id}/scan`.
    Request,
    /// A directory was added.
    DirectoryAdded,
    /// Files changed in a watched directory.
    Watcher,
}

/// How far a running scan got: the books its walk found, and how many of them it handled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ScanProgress {
    pub found: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub moved: usize,
    pub failed: usize,
}

/// A scan asked for that hasn't started: every request since the last one started, merged.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingScan {
    pub scope: ScanScope,
    pub triggers: BTreeSet<ScanTrigger>,
}

/// What a running scan does now: list the books, then read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    Walking,
    Reading,
}

/// The scan that runs now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunningScan {
    pub scope: ScanScope,
    pub triggers: BTreeSet<ScanTrigger>,
    /// Unix seconds.
    pub started_at: i64,
    pub phase: ScanPhase,
    #[serde(flatten)]
    pub progress: ScanProgress,
}

/// How a scan ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanOutcome {
    /// It ran to its end; what it couldn't read is among its failures.
    Completed,
    /// It stopped early — the directories couldn't be read, it panicked, or it was given up on
    /// when it stalled — and its counts are as far as it got.
    Failed,
}

/// The last scan that ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FinishedScan {
    pub scope: ScanScope,
    pub triggers: BTreeSet<ScanTrigger>,
    /// Unix seconds.
    pub started_at: i64,
    /// Unix seconds.
    pub finished_at: i64,
    pub outcome: ScanOutcome,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub moved: usize,
    pub removed: usize,
    /// Every failure; `failures` lists the first 100.
    pub failure_count: usize,
    pub failures: Vec<ScanFailure>,
}

/// Whether a scan runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanState {
    #[default]
    Idle,
    Running,
}

/// What `GET /api/v1/scan/status` answers: whether a scan runs and how far it got, the scan that
/// waits, and how the last one ended.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScanStatus {
    pub state: ScanState,
    pub current: Option<RunningScan>,
    pub pending: Option<PendingScan>,
    pub last: Option<FinishedScan>,
}
