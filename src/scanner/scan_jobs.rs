//! Scans as background jobs: one worker runs them one at a time and publishes how far it got. An
//! admin's scan is refused while another runs or waits; the server's own — for a directory just
//! added, a folder that changed — joins the one that waits. A scan that stops moving is given up
//! on, so the next one can run.

use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, SystemTime},
};

use tokio::{
    sync::{oneshot, watch},
    time::Instant,
};

use crate::{
    data_models::scan_model::{
        FinishedScan, PendingScan, RunningScan, ScanOutcome, ScanPhase, ScanProblem, ScanProgress,
        ScanReport, ScanScope, ScanState, ScanStatus, ScanTrigger,
    },
    db::{convert_system_time_to_unix_time, Database},
    error_types::{AppErrors, ScanError},
    scanner::{
        cover_image_retriever::Covers,
        folder_scanner,
        heartbeat::{Heartbeat, Spot},
    },
};

/// How many failures the last scan's status lists; the rest are only counted.
const LISTED_FAILURES: usize = 100;

/// How long a scan may show no sign of life before it is given up on: a healthy step beats within
/// seconds, while a file-system call on a share that stopped answering never returns.
pub const STALL_LIMIT: Duration = Duration::from_secs(2 * 60);

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
                    .run_next(STALL_LIMIT, move |scope, heartbeat, progress| {
                        folder_scanner::scan(&db, &covers, scope, heartbeat, progress)
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

    /// Runs the scan that waits, if one does, through `scan` on a thread of its own and publishes
    /// its progress; then records how it ended. One that fails or panics is recorded as failed,
    /// and so is one that shows no sign of life for `limit`: it is given up on and left behind.
    pub async fn run_next<F>(&self, limit: Duration, scan: F)
    where
        F: FnOnce(
                &ScanScope,
                &Heartbeat,
                &mut dyn FnMut(ScanProgress),
            ) -> Result<ScanReport, AppErrors>
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
        let heartbeat = Arc::new(Heartbeat::default());
        let (sender, mut result) = oneshot::channel();
        let publisher = Arc::clone(&self.status);
        let beating = Arc::clone(&heartbeat);
        // Not the blocking pool: a scan stuck on a dead share is left behind for good, and must not
        // keep a thread the database calls need.
        let started = std::thread::Builder::new()
            .name("scan".to_string())
            .spawn(move || {
                let report = scan(&scope, &beating, &mut |progress| {
                    publisher.send_if_modified(|status| {
                        // Checked under the status lock, so a scan given up on never writes over
                        // the next scan's progress.
                        if beating.given_up() {
                            return false;
                        }
                        let Some(current) = &mut status.current else {
                            return false;
                        };
                        current.phase = ScanPhase::Reading;
                        current.progress = progress;
                        true
                    });
                });
                // Nobody listens once the scan was given up on.
                let _ = sender.send(report);
            });
        let ending = match started {
            Ok(_) => {
                let mut watchdog = Watchdog::new(limit, heartbeat.beats(), Instant::now());
                loop {
                    tokio::select! {
                        ended = &mut result => break match ended {
                            Ok(Ok(report)) => Ending::Report(report),
                            Ok(Err(error)) => {
                                tracing::error!("a scan stopped: {error}");
                                Ending::Failed
                            }
                            Err(_) => {
                                tracing::error!("a scan panicked");
                                Ending::Failed
                            }
                        },
                        () = tokio::time::sleep(limit / 8) => {
                            if watchdog.stalled(heartbeat.beats(), Instant::now()) {
                                // Before the status is written, so its progress can't follow.
                                heartbeat.give_up();
                                let spot = heartbeat.spot();
                                tracing::error!(
                                    "a scan showed no sign of life for {limit:?} — stuck at {:?} — and was given up on",
                                    spot.as_ref().map(|spot| &spot.path)
                                );
                                break Ending::Stalled(spot);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                tracing::error!("a scan couldn't start: {error}");
                Ending::Failed
            }
        };
        self.status.send_modify(|status| {
            if let Some(running) = status.current.take() {
                status.state = ScanState::Idle;
                status.last = Some(finished(running, ending));
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

/// Tells a scan that stopped moving: one whose beats didn't change for `limit`.
struct Watchdog {
    limit: Duration,
    beats: u64,
    since: Instant,
}

impl Watchdog {
    fn new(limit: Duration, beats: u64, now: Instant) -> Watchdog {
        Watchdog {
            limit,
            beats,
            since: now,
        }
    }

    /// Whether the scan, at `beats` now, showed no sign of life for the whole limit.
    fn stalled(&mut self, beats: u64, now: Instant) -> bool {
        if beats != self.beats {
            self.beats = beats;
            self.since = now;
            return false;
        }
        now.duration_since(self.since) >= self.limit
    }
}

/// How a scan's run ended.
enum Ending {
    /// It ran to its end.
    Report(ScanReport),
    /// It stopped early, or panicked.
    Failed,
    /// It stopped moving and was given up on — where it got stuck, if it got that far.
    Stalled(Option<Spot>),
}

/// The record of `running` once it ended.
fn finished(running: RunningScan, ending: Ending) -> FinishedScan {
    let (outcome, progress, removed, mut failures) = match ending {
        Ending::Report(report) => (
            ScanOutcome::Completed,
            ScanProgress {
                found: running.progress.found,
                added: report.added,
                updated: report.updated,
                unchanged: report.unchanged,
                moved: report.moved,
                failed: report.failures.len(),
            },
            report.removed,
            report.failures,
        ),
        // Removals come after every book was read, so a scan stopped early made none.
        Ending::Failed => (ScanOutcome::Failed, running.progress, 0, Vec::new()),
        Ending::Stalled(spot) => {
            let stuck: Vec<_> = spot
                .iter()
                .map(|spot| spot.failure(ScanProblem::Stalled))
                .collect();
            let progress = ScanProgress {
                failed: running.progress.failed + stuck.len(),
                ..running.progress
            };
            (ScanOutcome::Failed, progress, 0, stuck)
        }
    };
    failures.truncate(LISTED_FAILURES);
    FinishedScan {
        scope: running.scope,
        triggers: running.triggers,
        started_at: running.started_at,
        finished_at: now(),
        outcome,
        added: progress.added,
        updated: progress.updated,
        unchanged: progress.unchanged,
        moved: progress.moved,
        removed,
        failure_count: progress.failed,
        failures,
    }
}

fn now() -> i64 {
    convert_system_time_to_unix_time(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::data_models::scan_model::{ScanFailure, ScanProblem};
    use crate::error_types::DatabaseError;
    use crate::test_support::{TempDatabase, TempDir};

    /// A report of `added` and `unchanged` books, and `failed` unreadable ones.
    fn report(added: usize, unchanged: usize, failed: usize) -> ScanReport {
        ScanReport {
            added,
            updated: 0,
            unchanged,
            moved: 0,
            removed: 0,
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

    fn folders(paths: &[&str]) -> ScanScope {
        ScanScope::Folders(paths.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn scopes_merge_into_one_that_reads_both_and_a_folder_inside_another_adds_nothing() {
        let manga = ScanScope::folder("/books/manga");

        assert_eq!(
            manga.clone().merge(ScanScope::folder("/books/comics")),
            folders(&["/books/comics", "/books/manga"])
        );
        assert_eq!(
            manga
                .clone()
                .merge(ScanScope::folder("/books/manga/Berserk")),
            manga
        );
        assert_eq!(
            ScanScope::folder("/books/manga/Berserk").merge(manga.clone()),
            manga
        );
        assert_eq!(
            folders(&["/books/manga/Berserk", "/books/manga/Pluto"])
                .merge(folders(&["/books/comics", "/books/manga"])),
            folders(&["/books/comics", "/books/manga"])
        );
        // Whole names only: a folder beside another with a longer name is not inside it.
        assert_eq!(
            manga.clone().merge(ScanScope::folder("/books/manga 2")),
            folders(&["/books/manga", "/books/manga 2"])
        );
        assert_eq!(manga.clone().merge(ScanScope::All), ScanScope::All);
        assert_eq!(ScanScope::All.merge(manga), ScanScope::All);
    }

    #[test]
    fn an_admins_scan_waits_alone_and_the_servers_own_joins_it() {
        let jobs = ScanJobs::without_worker();

        let shown = jobs.request(ScanScope::folder("/books/manga")).unwrap();
        assert_eq!(
            shown.pending,
            Some(PendingScan {
                scope: ScanScope::folder("/books/manga"),
                triggers: triggers(&[ScanTrigger::Request]),
            })
        );
        assert!(matches!(
            jobs.request(ScanScope::All),
            Err(ScanError::InProgress)
        ));
        jobs.queue(
            ScanScope::folder("/books/comics"),
            ScanTrigger::DirectoryAdded,
        );

        assert_eq!(
            jobs.status().pending,
            Some(PendingScan {
                scope: folders(&["/books/comics", "/books/manga"]),
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
            updated: 0,
            unchanged: 0,
            moved: 0,
            failed: 1,
        };

        jobs.run_next(STALL_LIMIT, move |scope, _, progress| {
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

        jobs.run_next(STALL_LIMIT, |_, _, _| Ok(report(0, 0, 150)))
            .await;

        let last = jobs.status().last.unwrap();
        assert_eq!((last.failure_count, last.failures.len()), (150, 100));
        assert_eq!(last.failures[99].relative_path, "Book 99.cbz");
    }

    #[tokio::test]
    async fn a_scan_that_panics_or_stops_is_recorded_as_failed_and_the_next_one_runs() {
        let jobs = ScanJobs::without_worker();
        let outcome = |jobs: &ScanJobs| jobs.status().last.unwrap().outcome;

        jobs.request(ScanScope::All).unwrap();
        jobs.run_next(STALL_LIMIT, |_, _, progress| {
            progress(ScanProgress {
                found: 4,
                added: 2,
                updated: 0,
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
        jobs.run_next(STALL_LIMIT, |_, _, _| {
            Err(DatabaseError::QueryFailure.into())
        })
        .await;
        assert_eq!(outcome(&jobs), ScanOutcome::Failed);

        jobs.request(ScanScope::All).unwrap();
        jobs.run_next(STALL_LIMIT, |_, _, _| Ok(report(1, 0, 0)))
            .await;
        assert_eq!(outcome(&jobs), ScanOutcome::Completed);
    }

    #[tokio::test]
    async fn nothing_runs_while_nothing_waits() {
        let jobs = ScanJobs::without_worker();

        jobs.run_next(STALL_LIMIT, |_, _, _| panic!("no scan was asked for"))
            .await;

        assert_eq!(jobs.status(), ScanStatus::default());
    }

    #[tokio::test]
    async fn the_worker_runs_a_queued_scan_by_itself() {
        let temp = TempDatabase::open();
        let books = TempDir::new();
        books.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO directories (path) VALUES (?1)",
                    [books.path().to_string_lossy()],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        let covers = Covers::in_folder(temp.covers.path(), None).unwrap();
        let jobs = ScanJobs::start(Arc::clone(&temp.db), covers);

        jobs.queue(ScanScope::folder(books.path()), ScanTrigger::DirectoryAdded);
        jobs.idle().await;

        let last = jobs.status().last.unwrap();
        assert_eq!((last.outcome, last.added), (ScanOutcome::Completed, 1));
        assert_eq!(last.triggers, triggers(&[ScanTrigger::DirectoryAdded]));
    }

    fn seconds(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    /// Waits for `run` — a scan meant to stall and be given up on — at most 10 s, so a watchdog
    /// that never gives up fails the test instead of hanging it.
    async fn given_up_on_in_time(run: impl std::future::Future<Output = ()>) {
        tokio::time::timeout(seconds(10), run)
            .await
            .expect("the stalled scan is given up on within 10 s");
    }

    #[test]
    fn a_scan_stalls_only_after_a_whole_limit_without_a_sign_of_life() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(seconds(120), 0, start);
        for beat in 1..=36 {
            assert!(
                !watchdog.stalled(beat, start + seconds(beat * 100)),
                "a beat every 100 s"
            );
        }
        let last = start + seconds(3600);

        assert!(!watchdog.stalled(36, last + seconds(119)));
        assert!(watchdog.stalled(36, last + seconds(120)));
        assert!(
            !watchdog.stalled(37, last + seconds(121)),
            "a beat starts over"
        );
        assert!(!watchdog.stalled(37, last + seconds(240)));
        assert!(watchdog.stalled(37, last + seconds(241)));
    }

    #[test]
    fn a_stalled_scan_ends_failed_with_where_it_got_stuck() {
        let running = RunningScan {
            scope: ScanScope::All,
            triggers: triggers(&[ScanTrigger::Request]),
            started_at: 1,
            phase: ScanPhase::Reading,
            progress: ScanProgress {
                found: 3,
                added: 1,
                failed: 2,
                ..ScanProgress::default()
            },
        };
        let spot = Spot {
            directory_id: 4,
            root: PathBuf::from("/books"),
            path: PathBuf::from("/books/Manga/Berserk v02.cbz"),
        };

        let ended = finished(running, Ending::Stalled(Some(spot)));

        assert_eq!(ended.outcome, ScanOutcome::Failed);
        assert_eq!((ended.added, ended.removed, ended.failure_count), (1, 0, 3));
        assert_eq!(
            ended.failures,
            [ScanFailure {
                directory_id: 4,
                relative_path: "Manga/Berserk v02.cbz".to_string(),
                error_key: ScanProblem::Stalled,
            }]
        );
    }

    #[tokio::test]
    async fn a_scan_that_stops_moving_is_given_up_on_and_the_next_one_runs() {
        let jobs = ScanJobs::without_worker();
        jobs.request(ScanScope::All).unwrap();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (told, given_up) = oneshot::channel();

        given_up_on_in_time(jobs.run_next(
            Duration::from_millis(100),
            move |_, heartbeat, progress| {
                heartbeat.in_directory(1, Path::new("/books"));
                heartbeat.at(Path::new("/books/Manga/Berserk v01.cbz"));
                progress(ScanProgress {
                    found: 3,
                    added: 1,
                    ..ScanProgress::default()
                });
                // A share that stopped answering.
                let _ = held.recv();
                let _ = told.send(heartbeat.check().is_err());
                Ok(ScanReport::default())
            },
        ))
        .await;

        let status = jobs.status();
        let last = status.last.unwrap();
        assert_eq!(status.state, ScanState::Idle);
        assert_eq!(
            (last.outcome, last.added, last.failure_count),
            (ScanOutcome::Failed, 1, 1)
        );
        assert_eq!(
            last.failures,
            [ScanFailure {
                directory_id: 1,
                relative_path: "Manga/Berserk v01.cbz".to_string(),
                error_key: ScanProblem::Stalled,
            }]
        );
        jobs.request(ScanScope::All).unwrap();
        jobs.run_next(Duration::from_millis(100), |_, _, _| Ok(report(1, 0, 0)))
            .await;
        assert_eq!(jobs.status().last.unwrap().outcome, ScanOutcome::Completed);

        release.send(()).unwrap();
        assert!(
            given_up.await.unwrap(),
            "the scan left behind learns it may write nothing more"
        );
    }

    #[tokio::test]
    async fn a_scan_given_up_on_never_writes_over_the_next_scans_progress() {
        let jobs = ScanJobs::without_worker();
        jobs.request(ScanScope::All).unwrap();
        let (release_first, first_held) = std::sync::mpsc::channel::<()>();
        let (first_told, first_given_up) = oneshot::channel();
        given_up_on_in_time(jobs.run_next(
            Duration::from_millis(100),
            move |_, heartbeat, progress| {
                let _ = first_held.recv();
                progress(ScanProgress {
                    found: 999,
                    ..ScanProgress::default()
                });
                let _ = first_told.send(heartbeat.given_up());
                Ok(ScanReport::default())
            },
        ))
        .await;
        jobs.request(ScanScope::All).unwrap();
        let (release_second, second_held) = std::sync::mpsc::channel::<()>();
        let second = jobs.clone();
        let running = tokio::spawn(async move {
            second
                .run_next(STALL_LIMIT, move |_, _, progress| {
                    progress(ScanProgress {
                        found: 2,
                        ..ScanProgress::default()
                    });
                    let _ = second_held.recv();
                    Ok(ScanReport::default())
                })
                .await;
        });
        let mut status = jobs.status.subscribe();
        let _ = status
            .wait_for(|status| {
                status
                    .current
                    .as_ref()
                    .is_some_and(|current| current.progress.found == 2)
            })
            .await;

        release_first.send(()).unwrap();
        assert!(first_given_up.await.unwrap());

        assert_eq!(
            jobs.status().current.unwrap().progress.found,
            2,
            "untouched by the scan left behind"
        );
        release_second.send(()).unwrap();
        running.await.unwrap();
    }
}
