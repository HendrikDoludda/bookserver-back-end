//! Scans as background jobs: one worker runs them one at a time and publishes how far it got. An
//! admin's scan is refused while another runs or waits; the server's own — for a directory just
//! added — joins the one that waits.

use std::{collections::BTreeSet, sync::Arc, time::SystemTime};

use tokio::sync::watch;

use crate::{
    data_models::scan_model::{
        FinishedScan, PendingScan, RunningScan, ScanOutcome, ScanPhase, ScanProgress, ScanReport,
        ScanScope, ScanState, ScanStatus, ScanTrigger,
    },
    db::{convert_system_time_to_unix_time, Database},
    error_types::{AppErrors, ScanError},
    scanner::{cover_image_retriever::Covers, folder_scanner},
};

/// How many failures the last scan's status lists; the rest are only counted.
const LISTED_FAILURES: usize = 100;

/// The handle to the scan worker. Its clones share the worker and its status.
#[derive(Clone)]
pub struct ScanJobs {
    status: Arc<watch::Sender<ScanStatus>>,
}

impl ScanJobs {
    /// Starts the worker that runs every scan of `db`'s directories, one at a time.
    pub fn start(db: Arc<Database>, covers: Covers) -> ScanJobs {
        let jobs = ScanJobs::without_worker();
        let worker = jobs.clone();
        tokio::spawn(async move {
            let mut status = worker.status.subscribe();
            loop {
                // Ends the read of the status here: holding it would block the writes below.
                let asked = status
                    .wait_for(|status| status.pending.is_some())
                    .await
                    .is_ok();
                if !asked {
                    return;
                }
                let (db, covers) = (Arc::clone(&db), covers.clone());
                worker
                    .run_next(move |scope, progress| {
                        folder_scanner::scan(&db, &covers, scope, progress)
                    })
                    .await;
            }
        });
        jobs
    }

    /// A handle without a worker: a scan asked for waits until `run_next` runs it. Tests use it
    /// to hold a scan back.
    pub fn without_worker() -> ScanJobs {
        let (status, _) = watch::channel(ScanStatus::default());
        ScanJobs {
            status: Arc::new(status),
        }
    }

    /// The status now.
    pub fn status(&self) -> ScanStatus {
        self.status.borrow().clone()
    }

    /// An admin's scan of `scope`: queued, and answered with the status that shows it waiting —
    /// unless a scan runs or waits already.
    pub fn request(&self, scope: ScanScope) -> Result<ScanStatus, ScanError> {
        let mut queued = Err(ScanError::InProgress);
        // Checked and queued in one step, so two requests at once can't both find it free.
        self.status.send_if_modified(|status| {
            if status.current.is_some() || status.pending.is_some() {
                return false;
            }
            status.pending = Some(PendingScan {
                scope,
                triggers: BTreeSet::from([ScanTrigger::Request]),
            });
            queued = Ok(status.clone());
            true
        });
        queued
    }

    /// The server's own scan of `scope`, for `trigger`: it joins the scan that waits.
    pub fn queue(&self, scope: ScanScope, trigger: ScanTrigger) {
        self.status.send_modify(|status| {
            status.pending = Some(match status.pending.take() {
                None => PendingScan {
                    scope,
                    triggers: BTreeSet::from([trigger]),
                },
                Some(waiting) => PendingScan {
                    scope: waiting.scope.merge(scope),
                    triggers: waiting.triggers.into_iter().chain([trigger]).collect(),
                },
            });
        });
    }

    /// Runs the scan that waits, if one does, through `scan` on the blocking pool and publishes
    /// its progress; then records how it ended. One that fails or panics is recorded as failed.
    pub async fn run_next<F>(&self, scan: F)
    where
        F: FnOnce(&ScanScope, &mut dyn FnMut(ScanProgress)) -> Result<ScanReport, AppErrors>
            + Send
            + 'static,
    {
        let mut scope = None;
        self.status.send_modify(|status| {
            if let Some(pending) = status.pending.take() {
                scope = Some(pending.scope.clone());
                status.state = ScanState::Running;
                status.current = Some(RunningScan {
                    scope: pending.scope,
                    triggers: pending.triggers,
                    started_at: now(),
                    phase: ScanPhase::Walking,
                    progress: ScanProgress::default(),
                });
            }
        });
        let Some(scope) = scope else {
            return;
        };
        let publisher = Arc::clone(&self.status);
        let ended = tokio::task::spawn_blocking(move || {
            scan(&scope, &mut |progress| {
                publisher.send_modify(|status| {
                    if let Some(current) = &mut status.current {
                        current.phase = ScanPhase::Reading;
                        current.progress = progress;
                    }
                });
            })
        })
        .await;
        let report = match ended {
            Ok(Ok(report)) => Some(report),
            Ok(Err(error)) => {
                tracing::error!("a scan stopped: {error}");
                None
            }
            Err(error) => {
                tracing::error!("a scan panicked: {error}");
                None
            }
        };
        self.status.send_modify(|status| {
            if let Some(running) = status.current.take() {
                status.state = ScanState::Idle;
                status.last = Some(finished(running, report));
            }
        });
    }

    /// Waits until no scan runs or waits.
    #[cfg(test)]
    pub async fn idle(&self) {
        let mut status = self.status.subscribe();
        // `self` holds the sender, so the wait can't fail.
        let _ = status
            .wait_for(|status| status.current.is_none() && status.pending.is_none())
            .await;
    }
}

/// The record of `running` once it ended: with its `report` when it ran to its end.
fn finished(running: RunningScan, report: Option<ScanReport>) -> FinishedScan {
    let (outcome, added, unchanged, failure_count, mut failures) = match report {
        Some(report) => (
            ScanOutcome::Completed,
            report.added,
            report.unchanged,
            report.failures.len(),
            report.failures,
        ),
        None => (
            ScanOutcome::Failed,
            running.progress.added,
            running.progress.unchanged,
            running.progress.failed,
            Vec::new(),
        ),
    };
    failures.truncate(LISTED_FAILURES);
    FinishedScan {
        scope: running.scope,
        triggers: running.triggers,
        started_at: running.started_at,
        finished_at: now(),
        outcome,
        added,
        unchanged,
        failure_count,
        failures,
    }
}

fn now() -> i64 {
    convert_system_time_to_unix_time(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_models::scan_model::{ScanFailure, ScanProblem};
    use crate::error_types::DatabaseError;
    use crate::test_support::{TempDatabase, TempDir};

    /// A report of `added` and `unchanged` books, and `failed` unreadable ones.
    fn report(added: usize, unchanged: usize, failed: usize) -> ScanReport {
        ScanReport {
            added,
            unchanged,
            failures: (0..failed)
                .map(|n| ScanFailure {
                    directory_id: 1,
                    relative_path: format!("Book {n}.cbz"),
                    error_key: ScanProblem::FileUnreadable,
                })
                .collect(),
        }
    }

    fn triggers(list: &[ScanTrigger]) -> BTreeSet<ScanTrigger> {
        list.iter().copied().collect()
    }

    #[test]
    fn scopes_merge_into_one_that_reads_both() {
        let one = ScanScope::directory(1);

        assert_eq!(
            one.clone().merge(ScanScope::directory(2)),
            ScanScope::Directories(BTreeSet::from([1, 2]))
        );
        assert_eq!(one.clone().merge(ScanScope::All), ScanScope::All);
        assert_eq!(ScanScope::All.merge(one.clone()), ScanScope::All);
        assert!(one.reads(1) && !one.reads(2) && ScanScope::All.reads(2));
    }

    #[test]
    fn an_admins_scan_waits_alone_and_the_servers_own_joins_it() {
        let jobs = ScanJobs::without_worker();

        let shown = jobs.request(ScanScope::directory(1)).unwrap();
        assert_eq!(
            shown.pending,
            Some(PendingScan {
                scope: ScanScope::directory(1),
                triggers: triggers(&[ScanTrigger::Request]),
            })
        );
        assert!(matches!(
            jobs.request(ScanScope::All),
            Err(ScanError::InProgress)
        ));
        jobs.queue(ScanScope::directory(2), ScanTrigger::DirectoryAdded);

        assert_eq!(
            jobs.status().pending,
            Some(PendingScan {
                scope: ScanScope::Directories(BTreeSet::from([1, 2])),
                triggers: triggers(&[ScanTrigger::Request, ScanTrigger::DirectoryAdded]),
            })
        );
    }

    #[tokio::test]
    async fn a_running_scan_shows_its_progress_and_refuses_an_admins_request() {
        let jobs = ScanJobs::without_worker();
        jobs.request(ScanScope::All).unwrap();
        let inside = jobs.clone();
        let progress_so_far = ScanProgress {
            found: 3,
            added: 1,
            unchanged: 0,
            failed: 1,
        };

        jobs.run_next(move |scope, progress| {
            assert_eq!(scope, &ScanScope::All);
            let running = inside.status();
            assert_eq!(running.state, ScanState::Running);
            assert!(running.pending.is_none());
            assert_eq!(running.current.unwrap().phase, ScanPhase::Walking);
            assert!(matches!(
                inside.request(ScanScope::All),
                Err(ScanError::InProgress)
            ));
            progress(progress_so_far);
            let current = inside.status().current.unwrap();
            assert_eq!(
                (current.phase, current.progress),
                (ScanPhase::Reading, progress_so_far)
            );
            Ok(report(2, 0, 1))
        })
        .await;

        let status = jobs.status();
        assert_eq!((status.state, status.current), (ScanState::Idle, None));
        let last = status.last.unwrap();
        assert_eq!(
            (last.outcome, last.added, last.unchanged, last.failure_count),
            (ScanOutcome::Completed, 2, 0, 1)
        );
        assert_eq!(last.failures, report(2, 0, 1).failures);
        assert!(last.finished_at >= last.started_at);
    }

    #[tokio::test]
    async fn the_last_scan_lists_its_first_hundred_failures_and_counts_them_all() {
        let jobs = ScanJobs::without_worker();
        jobs.request(ScanScope::All).unwrap();

        jobs.run_next(|_, _| Ok(report(0, 0, 150))).await;

        let last = jobs.status().last.unwrap();
        assert_eq!((last.failure_count, last.failures.len()), (150, 100));
        assert_eq!(last.failures[99].relative_path, "Book 99.cbz");
    }

    #[tokio::test]
    async fn a_scan_that_panics_or_stops_is_recorded_as_failed_and_the_next_one_runs() {
        let jobs = ScanJobs::without_worker();
        let outcome = |jobs: &ScanJobs| jobs.status().last.unwrap().outcome;

        jobs.request(ScanScope::All).unwrap();
        jobs.run_next(|_, progress| {
            progress(ScanProgress {
                found: 4,
                added: 2,
                ..ScanProgress::default()
            });
            panic!("a bug in the scan");
        })
        .await;
        let last = jobs.status().last.unwrap();
        assert_eq!(
            (last.outcome, last.added),
            (ScanOutcome::Failed, 2),
            "as far as it got"
        );
        assert_eq!(jobs.status().state, ScanState::Idle);

        jobs.request(ScanScope::All).unwrap();
        jobs.run_next(|_, _| Err(DatabaseError::QueryFailure.into()))
            .await;
        assert_eq!(outcome(&jobs), ScanOutcome::Failed);

        jobs.request(ScanScope::All).unwrap();
        jobs.run_next(|_, _| Ok(report(1, 0, 0))).await;
        assert_eq!(outcome(&jobs), ScanOutcome::Completed);
    }

    #[tokio::test]
    async fn nothing_runs_while_nothing_waits() {
        let jobs = ScanJobs::without_worker();

        jobs.run_next(|_, _| panic!("no scan was asked for")).await;

        assert_eq!(jobs.status(), ScanStatus::default());
    }

    #[tokio::test]
    async fn the_worker_runs_a_queued_scan_by_itself() {
        let temp = TempDatabase::open();
        let books = TempDir::new();
        books.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);
        let directory = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO directories (path) VALUES (?1)",
                    [books.path().to_string_lossy()],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap();
        let covers = Covers::in_folder(temp.covers.path(), None).unwrap();
        let jobs = ScanJobs::start(Arc::clone(&temp.db), covers);

        jobs.queue(ScanScope::directory(directory), ScanTrigger::DirectoryAdded);
        jobs.idle().await;

        let last = jobs.status().last.unwrap();
        assert_eq!((last.outcome, last.added), (ScanOutcome::Completed, 1));
        assert_eq!(last.triggers, triggers(&[ScanTrigger::DirectoryAdded]));
    }
}
