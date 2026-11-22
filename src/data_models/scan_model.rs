//! What a scan answers: the report of one run, and the status of the scan job.

use std::collections::BTreeSet;

use serde::Serialize;

/// What a scan did, as an admin gets it.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct ScanReport {
    /// Books this scan stored.
    pub added: usize,
    /// Books found stored already, left as they are.
    pub unchanged: usize,
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
}

/// Where a scan reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanScope {
    /// Every added directory.
    All,
    /// These directories.
    Directories(BTreeSet<i64>),
}

impl ScanScope {
    /// The directory `id` alone.
    pub fn directory(id: i64) -> ScanScope {
        ScanScope::Directories(BTreeSet::from([id]))
    }

    /// Whether the scope reads the directory `id`.
    pub fn reads(&self, id: i64) -> bool {
        match self {
            ScanScope::All => true,
            ScanScope::Directories(ids) => ids.contains(&id),
        }
    }

    /// One scope that reads what both read.
    pub fn merge(self, other: ScanScope) -> ScanScope {
        match (self, other) {
            (ScanScope::Directories(mut ids), ScanScope::Directories(more)) => {
                ids.extend(more);
                ScanScope::Directories(ids)
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
}

/// How far a running scan got: the books its walk found, and how many of them it handled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ScanProgress {
    pub found: usize,
    pub added: usize,
    pub unchanged: usize,
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
    /// It stopped early — the directories couldn't be read, or it panicked — and its counts are
    /// as far as it got.
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
    pub unchanged: usize,
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
