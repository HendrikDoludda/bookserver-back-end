//! Watching the added directories: a change queues a scan of the folder it happened in, once that
//! folder has rested, so a new file shows up without anyone asking for a scan. Every folder is
//! looked at each minute, so one that went away — an unplugged drive — is watched again once it is
//! back.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime},
};

use notify::{event::ModifyKind, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{
    sync::mpsc,
    time::{Instant, MissedTickBehavior},
};
use tracing::{debug, info, warn};

use crate::{
    data_models::scan_model::{ScanScope, ScanTrigger},
    scanner::{
        folder_scanner::{holds_anything, is_hidden},
        lock,
        scan_jobs::ScanJobs,
    },
};

/// How long a folder goes without a change before it is scanned: a copy rarely pauses this long
/// between two files.
const QUIET: Duration = Duration::from_secs(10);

/// How long a folder that keeps changing waits at most: a download running for an hour must not
/// hold back the books copied beside it.
const LONGEST: Duration = Duration::from_secs(5 * 60);

/// How long a call into a folder or its watcher may take before the server stops waiting: on a
/// share that stopped answering it never returns, while a healthy one answers in milliseconds.
const CALL_LIMIT: Duration = Duration::from_secs(30);

/// How often every folder is looked at: an unplugged drive plugged back in is watched again within
/// a minute, for one `stat` per folder.
const RECHECK: Duration = Duration::from_secs(60);

type Events = notify::Result<Event>;

/// The watchers on the added directories, one each, shared by the handlers that add and remove
/// them.
#[derive(Clone)]
pub struct FolderWatch {
    /// Where every watcher sends its events; `None` when nothing watches.
    events: Option<mpsc::UnboundedSender<Events>>,
    watched: Arc<Mutex<BTreeMap<i64, Watched>>>,
    tickets: Arc<AtomicU64>,
}

/// An added directory as the watch knows it.
struct Watched {
    root: PathBuf,
    /// New with each registration, so a check that ends late can tell it is out of date.
    ticket: u64,
    state: State,
    /// Present exactly while `state` is `Watching`; dropping it ends the watch.
    watcher: Option<RecommendedWatcher>,
    /// The system reported the root itself removed, renamed or unmounted since the last check.
    lost: bool,
    /// A check of it is under way: a folder that doesn't answer holds one thread, never more.
    checking: bool,
}

/// Where a directory's watch stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Not looked at yet.
    Waiting,
    /// Watched since the folder had this identity.
    Watching(Option<Identity>),
    /// The system refused to watch it while it had this identity.
    Refused(Option<Identity>),
    /// Its folder is missing, or an empty mount point.
    Gone,
}

/// Which folder a path names: another device or inode after an unmount and a mount, or after the
/// folder was replaced. `None` where the system can't tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

/// What a look at a directory's root found.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Seen {
    Missing,
    Present {
        identity: Option<Identity>,
        holds_anything: bool,
    },
}

/// What a check does with a directory's watch.
#[derive(Debug, PartialEq)]
enum Decision {
    Keep,
    /// Its folder is gone: its watch ends.
    Lose,
    /// Watch it anew — and scan it when `scan`, since changes made meanwhile went unseen.
    Watch {
        scan: bool,
    },
}

/// How a call under a time limit ended.
enum Call<T> {
    Returned(T),
    Panicked,
    TookTooLong,
}

impl FolderWatch {
    /// Watches `directories`, and starts the tasks that turn changes into scans and look at every
    /// folder each minute. Instant: the watches themselves start in the background.
    pub fn start(
        scans: ScanJobs,
        directories: impl IntoIterator<Item = (i64, PathBuf)>,
    ) -> FolderWatch {
        let (watch, events) = FolderWatch::with_events();
        for (id, root) in directories {
            watch.register(id, root, false);
        }
        tokio::spawn(drive(events, Arc::clone(&watch.watched), scans.clone()));
        tokio::spawn(keep_watching(watch.clone(), scans));
        watch
    }

    /// One that watches nothing: no directory reports a change.
    pub fn off() -> FolderWatch {
        FolderWatch {
            events: None,
            watched: Arc::default(),
            tickets: Arc::default(),
        }
    }

    /// A watch whose events go to the channel returned with it, which nothing reads yet.
    fn with_events() -> (FolderWatch, mpsc::UnboundedReceiver<Events>) {
        let (sender, events) = mpsc::unbounded_channel();
        let watch = FolderWatch {
            events: Some(sender),
            ..FolderWatch::off()
        };
        (watch, events)
    }

    /// Takes the directory `id` at `root` in, `checking` when a check of it starts at once;
    /// answers its ticket. Touches nothing on disk, so known before its first event can arrive.
    fn register(&self, id: i64, root: PathBuf, checking: bool) -> u64 {
        let ticket = self.tickets.fetch_add(1, Ordering::SeqCst);
        lock(&self.watched).insert(
            id,
            Watched {
                root,
                ticket,
                state: State::Waiting,
                watcher: None,
                lost: false,
                checking,
            },
        );
        ticket
    }

    /// Watches the directory `id` at `root`, every folder inside it included. Answers whether the
    /// system watches it, within `CALL_LIMIT`: Linux, for one, limits how many folders it watches.
    pub async fn watch(&self, id: i64, root: PathBuf) -> bool {
        if self.events.is_none() {
            return false;
        }
        let ticket = self.register(id, root, true);
        self.run_check(id, ticket, None).await
    }

    /// Stops watching the directory `id`.
    pub async fn unwatch(&self, id: i64) {
        let removed = lock(&self.watched).remove(&id);
        if let Some(watcher) = removed.and_then(|watched| watched.watcher) {
            // FSEvents' watcher joins a thread as it drops, and inotify's can panic.
            if let Call::TookTooLong = within(CALL_LIMIT, move || drop(watcher)).await {
                warn!("ending the watch of the directory {id} took too long; it is left behind");
            }
        }
    }

    /// Whether the system watches the directory `id`.
    pub fn watching(&self, id: i64) -> bool {
        lock(&self.watched)
            .get(&id)
            .is_some_and(|watched| matches!(watched.state, State::Watching(_)) && !watched.lost)
    }

    /// Marks every directory without a check under way as checked; answers each one's id and
    /// ticket.
    fn claim_idle(&self) -> Vec<(i64, u64)> {
        lock(&self.watched)
            .iter_mut()
            .filter(|(_, watched)| !watched.checking)
            .map(|(id, watched)| {
                watched.checking = true;
                (*id, watched.ticket)
            })
            .collect()
    }

    /// Checks the directory `id` once, within `CALL_LIMIT`, and queues a scan on `scans` of a
    /// folder that came back. Answers whether it is watched.
    async fn run_check(&self, id: i64, ticket: u64, scans: Option<ScanJobs>) -> bool {
        let watch = self.clone();
        let call = within(CALL_LIMIT, move || {
            let checked = panic::catch_unwind(AssertUnwindSafe(|| {
                watch.check_now(id, ticket, scans.as_ref())
            }));
            // Whatever happened, the next check may start.
            if let Some(watched) = lock(&watch.watched)
                .get_mut(&id)
                .filter(|watched| watched.ticket == ticket)
            {
                watched.checking = false;
            }
            checked.is_ok()
        })
        .await;
        match call {
            Call::Returned(true) => {}
            Call::Returned(false) | Call::Panicked => {
                warn!("checking the watch of the directory {id} panicked")
            }
            Call::TookTooLong => warn!(
                "the directory {id} didn't answer within {CALL_LIMIT:?}; its watch is checked again once it does"
            ),
        }
        self.watching(id)
    }

    /// Looks at the directory `id` and starts, keeps, renews or ends its watch, as `decide` says;
    /// a check whose `ticket` is out of date — the directory was removed or added anew meanwhile —
    /// changes nothing. The lock is never held while the file system or notify is called.
    fn check_now(&self, id: i64, ticket: u64, scans: Option<&ScanJobs>) {
        let Some(events) = &self.events else {
            return;
        };
        let Some((root, state, lost)) = lock(&self.watched)
            .get(&id)
            .map(|watched| (watched.root.clone(), watched.state, watched.lost))
        else {
            return;
        };
        let seen = look_at(&root);
        let decision = decide(state, lost, &seen);
        if decision == Decision::Keep {
            return;
        }
        // Out of the map before anything else happens: a lost event arriving meanwhile stays.
        let old = {
            let mut watched = lock(&self.watched);
            let Some(entry) = watched.get_mut(&id).filter(|entry| entry.ticket == ticket) else {
                return;
            };
            entry.state = if decision == Decision::Lose {
                State::Gone
            } else {
                State::Waiting
            };
            entry.lost = false;
            entry.watcher.take()
        };
        // Before the new one: inotify counts both against its limit.
        drop(old);
        let Decision::Watch { scan } = decision else {
            info!("the directory {root:?} is gone; it is watched again once it is back");
            return;
        };
        let identity = match seen {
            Seen::Present { identity, .. } => identity,
            Seen::Missing => None,
        };
        let started = notify::recommended_watcher(events.clone()).and_then(|mut watcher| {
            watcher
                .watch(&root, RecursiveMode::Recursive)
                .map(|()| watcher)
        });
        let (state, watcher) = match started {
            Ok(watcher) => (State::Watching(identity), Some(watcher)),
            Err(error) => {
                warn!("the directory {root:?} is not watched for changes: {error}");
                (State::Refused(identity), None)
            }
        };
        let rejected = {
            let mut watched = lock(&self.watched);
            match watched.get_mut(&id).filter(|entry| entry.ticket == ticket) {
                Some(entry) => {
                    entry.state = state;
                    entry.watcher = watcher;
                    None
                }
                // Removed or added anew meanwhile: this watch is out of date.
                None => watcher,
            }
        };
        let installed = rejected.is_none() && matches!(state, State::Watching(_));
        drop(rejected);
        if installed {
            info!("watching {root:?} for changes");
            if let (true, Some(scans)) = (scan, scans) {
                scans.queue(ScanScope::folder(root), ScanTrigger::Watcher);
            }
        }
    }
}

/// Runs `work` on the blocking pool and waits for it at most `limit`; past that, it is left behind.
async fn within<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Call<T> {
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(value)) => Call::Returned(value),
        Ok(Err(_)) => Call::Panicked,
        Err(_) => Call::TookTooLong,
    }
}

/// What the directory's root is now.
fn look_at(root: &Path) -> Seen {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => Seen::Present {
            identity: identity(&metadata),
            holds_anything: holds_anything(root),
        },
        _ => Seen::Missing,
    }
}

#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt;
    Some(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn identity(_: &fs::Metadata) -> Option<Identity> {
    None
}

/// What a check does with a directory's watch, given where it stands, whether the system reported
/// its root gone meanwhile, and what the root is now. A folder whose identity changed was replaced
/// or mounted anew: Linux's watch of it ended, so it is watched again — unless it is now an empty
/// mount point.
fn decide(state: State, lost: bool, seen: &Seen) -> Decision {
    let Seen::Present {
        identity,
        holds_anything,
    } = *seen
    else {
        return match state {
            State::Gone => Decision::Keep,
            _ => Decision::Lose,
        };
    };
    let changed = match state {
        State::Waiting => return Decision::Watch { scan: false },
        State::Gone => {
            return if holds_anything {
                Decision::Watch { scan: true }
            } else {
                Decision::Keep
            }
        }
        State::Watching(was) => lost || was != identity,
        // Refused again and again otherwise, with a warning every minute.
        State::Refused(was) => was != identity,
    };
    match (changed, holds_anything) {
        (false, _) => Decision::Keep,
        (true, true) => Decision::Watch { scan: true },
        (true, false) => Decision::Lose,
    }
}

/// Every `RECHECK`, checks each directory no check is under way for — each on its own, so one that
/// doesn't answer holds back no other.
async fn keep_watching(watch: FolderWatch, scans: ScanJobs) {
    let mut ticks = tokio::time::interval(RECHECK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        for (id, ticket) in watch.claim_idle() {
            let (watch, scans) = (watch.clone(), scans.clone());
            tokio::spawn(async move { watch.run_check(id, ticket, Some(scans)).await });
        }
    }
}

/// Whether `event` reports the folder at `root` itself removed, renamed or unmounted: on Linux its
/// watch ended then, even when a folder of that name is back by the next check.
fn ends_watch(event: &Event, root: &Path) -> bool {
    matches!(
        event.kind,
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    ) && event.paths.iter().any(|path| path == root)
}

/// Turns the watchers' events into scans: each folder that changed is queued once it rested.
async fn drive(
    mut events: mpsc::UnboundedReceiver<Events>,
    watched: Arc<Mutex<BTreeMap<i64, Watched>>>,
    scans: ScanJobs,
) {
    let mut debounce = Debounce::default();
    loop {
        let wake = debounce.next_due();
        tokio::select! {
            received = events.recv() => match received {
                Some(Ok(event)) => {
                    let roots: Vec<PathBuf> = {
                        let mut watched = lock(&watched);
                        for entry in watched.values_mut() {
                            if ends_watch(&event, &entry.root) {
                                entry.lost = true;
                            }
                        }
                        watched.values().map(|entry| entry.root.clone()).collect()
                    };
                    let now = Instant::now();
                    for folder in folders_of(&event, &roots) {
                        debounce.record(folder, now);
                    }
                }
                Some(Err(error)) => warn!("watching the directories: {error}"),
                // Every watcher is gone.
                None => return,
            },
            () = sleep_until(wake) => {
                let now = Instant::now();
                let changing = still_changing(debounce.to_check(now), CALL_LIMIT, changed_recently).await;
                let due = debounce.due(now, |folder| changing.contains(folder));
                if let Some(scope) = due.into_iter().map(ScanScope::folder).reduce(ScanScope::merge) {
                    scans.queue(scope, ScanTrigger::Watcher);
                }
            }
        }
    }
}

/// Which of `folders` `check` finds changing, asked on the blocking pool within `limit`. Folders
/// that don't answer count as resting: their scan meets the dead share and is given up on, where
/// counting them as changing would start a new hung check every 10 s.
async fn still_changing(
    folders: Vec<PathBuf>,
    limit: Duration,
    check: fn(&Path) -> bool,
) -> BTreeSet<PathBuf> {
    if folders.is_empty() {
        return BTreeSet::new();
    }
    match within(limit, move || {
        folders.into_iter().filter(|folder| check(folder)).collect()
    })
    .await
    {
        Call::Returned(changing) => changing,
        Call::Panicked | Call::TookTooLong => BTreeSet::new(),
    }
}

/// Sleeps until `wake`, or for ever without one.
async fn sleep_until(wake: Option<Instant>) {
    match wake {
        Some(wake) => tokio::time::sleep_until(wake).await,
        None => std::future::pending().await,
    }
}

/// The folders a scan reads for `event`, each in one of the directories at `roots`: the folder
/// that holds each changed entry, so a new folder is read whole. When events were lost, the
/// directories they were lost in — or every one, when the system can't say which.
fn folders_of(event: &Event, roots: &[PathBuf]) -> Vec<PathBuf> {
    if event.need_rescan() {
        let named: Vec<PathBuf> = roots
            .iter()
            .filter(|root| {
                event
                    .paths
                    .iter()
                    .any(|path| path.starts_with(root) || root.starts_with(path))
            })
            .cloned()
            .collect();
        return if named.is_empty() {
            roots.to_vec()
        } else {
            named
        };
    }
    // Opening a file, or a Finder tag set on it, changes nothing a scan reads.
    if matches!(
        event.kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_))
    ) {
        return Vec::new();
    }
    event
        .paths
        .iter()
        .filter_map(|path| folder_of(path, roots))
        .collect()
}

/// The folder that holds `path`, or the root when `path` is one. `None` when it lies in no
/// directory, is hidden or inside a hidden folder, is a download still running (`.part`), or
/// isn't UTF-8 — a scan's scope goes out as JSON text.
fn folder_of(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let root = roots.iter().find(|root| path.starts_with(root))?;
    let inside = path.strip_prefix(root).ok()?;
    let skipped = path.to_str().is_none()
        || inside.iter().any(is_hidden)
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("part"));
    if skipped {
        debug!("ignoring the change at {path:?}");
        return None;
    }
    match path.parent() {
        Some(parent) if path != root => Some(parent.to_path_buf()),
        _ => Some(root.clone()),
    }
}

/// Whether something in `folder` changed within the last `QUIET`: a file still being copied.
fn changed_recently(folder: &Path) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    let now = SystemTime::now();
    entries.flatten().any(|entry| {
        !is_hidden(&entry.file_name())
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age < QUIET))
    })
}

/// The folders that changed and wait to be scanned.
#[derive(Debug, Default)]
struct Debounce {
    waiting: BTreeMap<PathBuf, Waiting>,
}

/// When a waiting folder first changed, and when it last did.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Waiting {
    first: Instant,
    last: Instant,
}

impl Waiting {
    fn due_at(&self) -> Instant {
        (self.last + QUIET).min(self.first + LONGEST)
    }
}

impl Debounce {
    /// Notes a change in `folder` at `now`. It joins a folder waiting above it, and takes in the
    /// ones waiting inside it: one scan reads them all.
    fn record(&mut self, folder: PathBuf, now: Instant) {
        if let Some(above) = self
            .waiting
            .iter_mut()
            .find_map(|(waiting, times)| folder.starts_with(waiting).then_some(times))
        {
            above.last = now;
            return;
        }
        let mut times = Waiting {
            first: now,
            last: now,
        };
        self.waiting.retain(|waiting, inside| {
            let absorbed = waiting.starts_with(&folder);
            if absorbed {
                times.first = times.first.min(inside.first);
            }
            !absorbed
        });
        self.waiting.insert(folder, times);
    }

    /// The folders `due` asks about at `now`: due, but not yet waiting `LONGEST`.
    fn to_check(&self, now: Instant) -> Vec<PathBuf> {
        self.waiting
            .iter()
            .filter(|(_, times)| times.due_at() <= now && now < times.first + LONGEST)
            .map(|(folder, _)| folder.clone())
            .collect()
    }

    /// When the next folder is due, if one waits.
    fn next_due(&self) -> Option<Instant> {
        self.waiting.values().map(Waiting::due_at).min()
    }

    /// Takes the folders due at `now`: quiet for `QUIET`, or waiting for `LONGEST`. One that
    /// `still_changing` finds changed lately waits once more, unless it waited `LONGEST`.
    fn due(&mut self, now: Instant, still_changing: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        let ready: Vec<PathBuf> = self
            .waiting
            .iter()
            .filter(|(_, times)| times.due_at() <= now)
            .map(|(folder, _)| folder.clone())
            .collect();
        let mut due = Vec::new();
        for folder in ready {
            let times = self.waiting[&folder];
            if now < times.first + LONGEST && still_changing(&folder) {
                self.waiting.insert(folder, Waiting { last: now, ..times });
            } else {
                self.waiting.remove(&folder);
                due.push(folder);
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use notify::event::{AccessKind, CreateKind, Flag, MetadataKind, RemoveKind, RenameMode};

    use super::*;
    use crate::{data_models::scan_model::PendingScan, test_support::TempDir};

    fn event(kind: EventKind, path: &Path) -> Event {
        Event::new(kind).add_path(path.to_path_buf())
    }

    fn created(path: &Path) -> Event {
        event(EventKind::Create(CreateKind::Any), path)
    }

    fn never_changing(_: &Path) -> bool {
        false
    }

    fn seconds(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    /// `dir`'s canonical path, the form a directory is stored and watched in.
    fn canonical(dir: &TempDir) -> PathBuf {
        fs::create_dir_all(dir.path()).unwrap();
        fs::canonicalize(dir.path()).unwrap()
    }

    /// Sets `path`'s modification time an hour back: a file that rests.
    fn rested(path: &Path) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - seconds(3600))
            .unwrap();
    }

    #[test]
    fn a_folder_is_due_once_it_went_ten_seconds_without_a_change() {
        let start = Instant::now();
        let mut debounce = Debounce::default();
        debounce.record(PathBuf::from("/books/Manga"), start);
        debounce.record(PathBuf::from("/books/Manga"), start + seconds(4));

        assert_eq!(debounce.next_due(), Some(start + seconds(14)));
        assert!(debounce.due(start + seconds(13), never_changing).is_empty());
        assert_eq!(
            debounce.due(start + seconds(14), never_changing),
            [PathBuf::from("/books/Manga")]
        );
        assert_eq!(debounce.next_due(), None);
    }

    #[test]
    fn a_folder_that_keeps_changing_is_due_five_minutes_after_its_first_change() {
        let start = Instant::now();
        let mut debounce = Debounce::default();
        let downloads = PathBuf::from("/books/Downloads");
        for second in (0..300).step_by(5) {
            debounce.record(downloads.clone(), start + seconds(second));
            let due = debounce.due(start + seconds(second), never_changing);
            assert!(due.is_empty(), "after {second} s");
        }

        debounce.record(downloads.clone(), start + seconds(300));
        assert_eq!(
            debounce.due(start + seconds(300), never_changing),
            std::slice::from_ref(&downloads)
        );
        debounce.record(downloads.clone(), start + seconds(305));
        assert!(
            debounce
                .due(start + seconds(305), never_changing)
                .is_empty(),
            "a change after the scan waits anew"
        );
    }

    #[test]
    fn a_folder_takes_in_the_folders_inside_it_and_joins_the_one_above_it() {
        let start = Instant::now();
        let mut debounce = Debounce::default();
        debounce.record(PathBuf::from("/books/Manga/Berserk"), start);
        debounce.record(PathBuf::from("/books/Manga 2"), start);
        debounce.record(PathBuf::from("/books/Manga"), start + seconds(3));
        debounce.record(PathBuf::from("/books/Manga/Pluto"), start + seconds(5));

        assert_eq!(
            debounce.due(start + seconds(14), never_changing),
            [PathBuf::from("/books/Manga 2")]
        );
        assert!(
            debounce.due(start + seconds(14), never_changing).is_empty(),
            "its last change came at 5 s"
        );
        assert_eq!(
            debounce.due(start + seconds(15), never_changing),
            [PathBuf::from("/books/Manga")]
        );

        for second in (0..300).step_by(5) {
            debounce.record(PathBuf::from("/books/Comics/Saga"), start + seconds(second));
            if second == 5 {
                debounce.record(PathBuf::from("/books/Comics"), start + seconds(second));
            }
        }
        assert_eq!(
            debounce.due(start + seconds(300), never_changing),
            [PathBuf::from("/books/Comics")],
            "five minutes after the first change inside it"
        );
    }

    #[test]
    fn a_folder_still_being_written_to_waits_again_but_not_past_five_minutes() {
        let start = Instant::now();
        let mut debounce = Debounce::default();
        debounce.record(PathBuf::from("/books/Manga"), start);
        let always_changing = |_: &Path| true;

        assert!(debounce
            .due(start + seconds(10), always_changing)
            .is_empty());
        assert_eq!(debounce.next_due(), Some(start + seconds(20)));
        assert!(debounce
            .due(start + seconds(290), always_changing)
            .is_empty());
        assert_eq!(debounce.next_due(), Some(start + seconds(300)));
        assert_eq!(
            debounce.due(start + seconds(300), always_changing),
            [PathBuf::from("/books/Manga")]
        );
    }

    #[test]
    fn a_change_asks_for_the_folder_that_holds_it_and_a_change_of_the_root_for_the_root() {
        let roots = [PathBuf::from("/books"), PathBuf::from("/comics")];
        let folders = |event: Event| folders_of(&event, &roots);

        assert_eq!(
            folders(created(Path::new("/books/Manga/Berserk v01.cbz"))),
            [PathBuf::from("/books/Manga")]
        );
        assert_eq!(
            folders(created(Path::new("/books/Manga"))),
            [PathBuf::from("/books")],
            "a new folder is read with the one holding it"
        );
        assert_eq!(
            folders(event(
                EventKind::Remove(RemoveKind::Any),
                Path::new("/comics/Saga v01.cbz")
            )),
            [PathBuf::from("/comics")]
        );
        assert_eq!(
            folders(event(
                EventKind::Modify(ModifyKind::Any),
                Path::new("/comics")
            )),
            [PathBuf::from("/comics")]
        );
        let renamed = event(
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)),
            Path::new("/books/Manga/old.cbz"),
        )
        .add_path(PathBuf::from("/comics/new.cbz"));
        assert_eq!(
            folders(renamed),
            [PathBuf::from("/books/Manga"), PathBuf::from("/comics")]
        );
    }

    #[test]
    fn reading_tagging_hidden_entries_downloads_and_other_folders_ask_for_nothing() {
        let roots = [PathBuf::from("/books")];
        let book = Path::new("/books/Manga/Berserk v01.cbz");

        let mut events = vec![
            event(EventKind::Access(AccessKind::Any), book),
            event(
                EventKind::Modify(ModifyKind::Metadata(MetadataKind::Extended)),
                book,
            ),
            created(Path::new("/books/Manga/.DS_Store")),
            created(Path::new("/books/.Trashes/501/Berserk v01.cbz")),
            created(Path::new(
                "/books/@eaDir/Berserk v01.cbz/SYNOFILE_THUMB_M.jpg",
            )),
            created(Path::new("/books/Manga/Berserk v02.cbz.part")),
            created(Path::new("/books/Manga/Berserk v02.PART")),
            created(Path::new("/booksellers/Berserk v01.cbz")),
            created(Path::new("/elsewhere/Berserk v01.cbz")),
        ];
        #[cfg(unix)]
        {
            use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
            events.push(created(
                &Path::new("/books").join(OsStr::from_bytes(b"caf\xe9.cbz")),
            ));
        }
        #[cfg(windows)]
        {
            use std::{ffi::OsString, os::windows::ffi::OsStringExt};
            // A lone surrogate (0xD800) is UTF-16 that isn't text; NTFS names may hold one.
            let name: Vec<u16> = "caf"
                .encode_utf16()
                .chain([0xD800])
                .chain(".cbz".encode_utf16())
                .collect();
            events.push(created(
                &Path::new("/books").join(OsString::from_wide(&name)),
            ));
        }
        for ignored in events {
            assert!(folders_of(&ignored, &roots).is_empty(), "{ignored:?}");
        }
    }

    #[test]
    fn lost_events_ask_for_the_whole_directory_they_were_lost_in_or_every_one() {
        let roots = [PathBuf::from("/books"), PathBuf::from("/comics")];
        let rescan = |path: Option<&str>| {
            let event = Event::new(EventKind::Other).set_flag(Flag::Rescan);
            let event = match path {
                Some(path) => event.add_path(PathBuf::from(path)),
                None => event,
            };
            folders_of(&event, &roots)
        };

        assert_eq!(
            rescan(Some("/books/Manga/Berserk")),
            [PathBuf::from("/books")]
        );
        assert_eq!(rescan(Some("/")), roots, "lost above every directory");
        assert_eq!(rescan(None), roots);
    }

    #[test]
    fn something_in_a_folder_changed_lately_until_it_rests() {
        let dir = TempDir::new();
        let book = dir.write("Manga/Berserk v01.cbz", b"archive");
        dir.write("Manga/.DS_Store", b"finder");
        let folder = dir.path().join("Manga");
        rested(&folder.join(".DS_Store"));

        assert!(changed_recently(&folder));
        rested(&book);
        dir.write("Manga/.DS_Store", b"finder again");
        assert!(!changed_recently(&folder), "a hidden file doesn't count");
        assert!(!changed_recently(&dir.path().join("Gone")));
    }

    /// Runs the task that turns events into scans, on the directory at `root` with the id 1,
    /// queueing into a `ScanJobs` without a worker; answers the channel it reads and the jobs.
    fn driven(root: &Path) -> (mpsc::UnboundedSender<Events>, ScanJobs, FolderWatch) {
        let scans = ScanJobs::without_worker();
        let watch = FolderWatch::off();
        watch.register(1, root.to_path_buf(), false);
        let (sender, events) = mpsc::unbounded_channel();
        tokio::spawn(drive(events, Arc::clone(&watch.watched), scans.clone()));
        (sender, scans, watch)
    }

    fn by_the_watcher(folder: PathBuf) -> Option<PendingScan> {
        Some(PendingScan {
            scope: ScanScope::folder(folder),
            triggers: BTreeSet::from([ScanTrigger::Watcher]),
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_changed_folder_is_queued_for_a_scan_once_it_rested() {
        let dir = TempDir::new();
        let book = dir.write("Manga/Berserk v01.cbz", b"archive");
        rested(&book);
        let root = canonical(&dir);
        let (sender, scans, _) = driven(&root);

        sender
            .send(Ok(created(&root.join("Manga/Berserk v01.cbz"))))
            .unwrap();
        sender
            .send(Ok(event(
                EventKind::Access(AccessKind::Any),
                &root.join("Comics/Saga v01.cbz"),
            )))
            .unwrap();
        tokio::time::sleep(seconds(9)).await;
        assert_eq!(scans.status().pending, None);
        tokio::time::sleep(seconds(2)).await;

        assert_eq!(scans.status().pending, by_the_watcher(root.join("Manga")));
    }

    #[tokio::test(start_paused = true)]
    async fn a_folder_still_being_written_to_is_queued_after_five_minutes_at_the_latest() {
        let dir = TempDir::new();
        dir.write("Downloads/Berserk v01.cbz", b"half an archive");
        let root = canonical(&dir);
        let (sender, scans, _) = driven(&root);

        sender
            .send(Ok(created(&root.join("Downloads/Berserk v01.cbz"))))
            .unwrap();
        tokio::time::sleep(seconds(299)).await;
        assert_eq!(scans.status().pending, None, "its file keeps a fresh time");
        tokio::time::sleep(seconds(2)).await;

        assert_eq!(
            scans.status().pending,
            by_the_watcher(root.join("Downloads"))
        );
    }

    #[tokio::test]
    async fn the_system_reports_a_file_dropped_into_a_watched_directory() {
        let dir = TempDir::new();
        fs::create_dir_all(dir.path().join("Manga")).unwrap();
        let root = canonical(&dir);
        let (watch, mut events) = FolderWatch::with_events();

        assert!(watch.watch(1, root.clone()).await);
        assert!(watch.watching(1));
        fs::write(root.join("Manga/Berserk v01.cbz"), b"archive").unwrap();
        let reported = tokio::time::timeout(seconds(20), async {
            while let Some(event) = events.recv().await {
                let event = event.expect("the watcher reports no error");
                if folders_of(&event, std::slice::from_ref(&root)) == [root.join("Manga")] {
                    return true;
                }
            }
            false
        })
        .await;
        assert_eq!(reported, Ok(true), "the event within 20 s");

        watch.unwatch(1).await;
        assert!(!watch.watching(1));
    }

    #[tokio::test]
    async fn without_a_watcher_nothing_is_watched() {
        let dir = TempDir::new();
        let watch = FolderWatch::off();

        assert!(!watch.watch(1, canonical(&dir)).await);
        assert!(!watch.watching(1));
        watch.unwatch(1).await;
        assert!(!watch.watching(1));
    }

    /// The path beside `root` that it is renamed to while it is away.
    fn away_from(root: &Path) -> PathBuf {
        let mut name = root.file_name().unwrap().to_os_string();
        name.push("-away");
        root.with_file_name(name)
    }

    #[tokio::test]
    async fn a_call_that_takes_too_long_is_left_behind_and_ends_on_its_own() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (finished, ended) = tokio::sync::oneshot::channel();

        let call = within(Duration::from_millis(20), move || {
            let _ = held.recv();
            let _ = finished.send(());
        })
        .await;

        assert!(matches!(call, Call::TookTooLong));
        release.send(()).unwrap();
        ended.await.unwrap();
        assert!(matches!(within(seconds(5), || 7).await, Call::Returned(7)));
        assert!(matches!(
            within(seconds(5), || -> u8 { panic!("a watcher broke") }).await,
            Call::Panicked
        ));
    }

    #[tokio::test]
    async fn folders_that_dont_answer_in_time_count_as_resting() {
        fn changing(_: &Path) -> bool {
            true
        }
        fn too_slow(_: &Path) -> bool {
            std::thread::sleep(Duration::from_millis(300));
            true
        }
        let folders = vec![PathBuf::from("/books/Manga")];

        assert_eq!(
            still_changing(folders.clone(), seconds(5), changing).await,
            BTreeSet::from([PathBuf::from("/books/Manga")])
        );
        assert!(still_changing(folders, Duration::from_millis(20), too_slow)
            .await
            .is_empty());
    }

    #[test]
    fn only_folders_due_and_not_yet_waiting_five_minutes_are_looked_into() {
        let start = Instant::now();
        let mut debounce = Debounce::default();
        debounce.record(PathBuf::from("/books/Old"), start);
        debounce.record(PathBuf::from("/books/Quiet"), start + seconds(290));
        debounce.record(PathBuf::from("/books/Busy"), start + seconds(295));

        assert_eq!(
            debounce.to_check(start + seconds(300)),
            [PathBuf::from("/books/Quiet")]
        );
    }

    #[test]
    fn only_a_removal_rename_or_unmount_of_the_root_itself_ends_its_watch() {
        let root = Path::new("/books");

        for ending in [
            EventKind::Remove(RemoveKind::Folder),
            EventKind::Remove(RemoveKind::Other),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        ] {
            assert!(ends_watch(&event(ending, root), root), "{ending:?}");
        }
        assert!(!ends_watch(
            &event(
                EventKind::Remove(RemoveKind::File),
                &root.join("Berserk v01.cbz")
            ),
            root
        ));
        assert!(!ends_watch(&created(root), root));
        assert!(!ends_watch(
            &event(EventKind::Access(AccessKind::Any), root),
            root
        ));
        assert!(!ends_watch(
            &event(EventKind::Remove(RemoveKind::Folder), Path::new("/books 2")),
            root
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn the_roots_own_removal_marks_its_watch_lost_and_a_change_inside_does_not() {
        let dir = TempDir::new();
        let root = canonical(&dir);
        let (sender, _, watch) = driven(&root);
        let lost = || lock(&watch.watched)[&1].lost;
        lock(&watch.watched).get_mut(&1).unwrap().state = State::Watching(None);

        sender
            .send(Ok(event(
                EventKind::Remove(RemoveKind::Any),
                &root.join("Manga/Berserk v01.cbz"),
            )))
            .unwrap();
        tokio::time::sleep(seconds(1)).await;
        assert!(!lost() && watch.watching(1));

        // How inotify reports an unmount.
        sender
            .send(Ok(event(EventKind::Remove(RemoveKind::Other), &root)))
            .unwrap();
        tokio::time::sleep(seconds(1)).await;
        assert!(lost() && !watch.watching(1));
    }

    #[test]
    fn a_check_decides_by_where_the_watch_stands_and_what_the_root_is_now() {
        use Decision::{Keep, Lose, Watch};
        let one = Some(Identity {
            device: 1,
            inode: 2,
        });
        let other = Some(Identity {
            device: 1,
            inode: 3,
        });
        let full = |identity| Seen::Present {
            identity,
            holds_anything: true,
        };
        let empty = |identity| Seen::Present {
            identity,
            holds_anything: false,
        };
        let missing = Seen::Missing;
        let cases = [
            (State::Waiting, false, missing, Lose),
            (State::Waiting, false, full(one), Watch { scan: false }),
            (State::Waiting, false, empty(one), Watch { scan: false }),
            (State::Watching(one), false, missing, Lose),
            (State::Watching(one), false, full(one), Keep),
            (State::Watching(one), true, full(one), Watch { scan: true }),
            (
                State::Watching(one),
                false,
                full(other),
                Watch { scan: true },
            ),
            (State::Watching(one), false, empty(one), Keep),
            (State::Watching(one), true, empty(one), Lose),
            (State::Watching(one), false, empty(other), Lose),
            (State::Refused(one), false, missing, Lose),
            (State::Refused(one), false, full(one), Keep),
            (State::Refused(one), true, full(one), Keep),
            (
                State::Refused(one),
                false,
                full(other),
                Watch { scan: true },
            ),
            (State::Refused(one), false, empty(one), Keep),
            (State::Refused(one), false, empty(other), Lose),
            (State::Gone, false, missing, Keep),
            (State::Gone, false, full(one), Watch { scan: true }),
            (State::Gone, false, empty(one), Keep),
        ];
        for (state, lost, seen, decision) in cases {
            assert_eq!(
                decide(state, lost, &seen),
                decision,
                "{state:?}, lost {lost}, {seen:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_look_at_a_root_tells_missing_empty_full_and_replaced_apart() {
        let dir = TempDir::new();
        let root = canonical(&dir);
        let Seen::Present {
            identity,
            holds_anything: false,
        } = look_at(&root)
        else {
            panic!("an empty folder");
        };
        fs::write(root.join("Berserk v01.cbz"), b"cbz").unwrap();

        assert_eq!(
            look_at(&root),
            Seen::Present {
                identity,
                holds_anything: true
            }
        );
        assert_eq!(
            look_at(&root.join("Berserk v01.cbz")),
            Seen::Missing,
            "a file"
        );
        assert_eq!(look_at(&root.join("Gone")), Seen::Missing);
        let away = away_from(&root);
        fs::rename(&root, &away).unwrap();
        fs::create_dir(&root).unwrap();
        let Seen::Present {
            identity: replaced, ..
        } = look_at(&root)
        else {
            panic!("a new folder of that name");
        };
        assert!(identity.is_some());
        assert_ne!(replaced, identity);
        fs::remove_dir_all(&away).unwrap();
    }

    #[tokio::test]
    async fn a_check_that_is_out_of_date_changes_nothing() {
        let dir = TempDir::new();
        let root = canonical(&dir);
        fs::write(root.join("Berserk v01.cbz"), b"cbz").unwrap();
        let (watch, _events) = FolderWatch::with_events();
        let check = |ticket| {
            let watch = watch.clone();
            tokio::task::spawn_blocking(move || watch.check_now(1, ticket, None))
        };
        // A check still under way when the directory is removed and added again under its id.
        let old = watch.register(1, root.clone(), false);
        watch.unwatch(1).await;
        let current = watch.register(1, root.clone(), false);
        check(current).await.unwrap();
        assert!(watch.watching(1));
        // Reported gone meanwhile, so a check that counted would watch it anew.
        lock(&watch.watched).get_mut(&1).unwrap().lost = true;

        check(old).await.unwrap();

        {
            let watched = lock(&watch.watched);
            assert!(matches!(watched[&1].state, State::Watching(_)));
            assert!(watched[&1].watcher.is_some(), "the current watch stays");
        }
        watch.unwatch(1).await;
        check(current).await.unwrap();
        assert!(lock(&watch.watched).is_empty(), "removed since");
    }

    /// The watch on the directory at `root` with the id 1, looked at every minute; queues into a
    /// `ScanJobs` without a worker.
    fn kept_watching(root: &Path) -> (FolderWatch, ScanJobs, mpsc::UnboundedReceiver<Events>) {
        let scans = ScanJobs::without_worker();
        let (watch, events) = FolderWatch::with_events();
        watch.register(1, root.to_path_buf(), false);
        tokio::spawn(keep_watching(watch.clone(), scans.clone()));
        (watch, scans, events)
    }

    #[tokio::test(start_paused = true)]
    async fn a_folder_that_goes_away_is_unwatched_then_watched_and_scanned_once_back() {
        let dir = TempDir::new();
        let root = canonical(&dir);
        let away = away_from(&root);
        let (watch, scans, _events) = kept_watching(&root);

        tokio::time::sleep(seconds(1)).await;
        assert!(watch.watching(1), "watched in the first round");
        assert_eq!(scans.status().pending, None);
        fs::rename(&root, &away).unwrap();
        tokio::time::sleep(RECHECK).await;
        assert!(!watch.watching(1), "gone");
        fs::rename(&away, &root).unwrap();
        fs::write(root.join("Berserk v01.cbz"), b"cbz").unwrap();
        tokio::time::sleep(RECHECK).await;

        assert!(watch.watching(1), "back");
        assert_eq!(scans.status().pending, by_the_watcher(root.clone()));
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn a_watched_folder_replaced_between_two_looks_is_watched_anew_and_scanned() {
        let dir = TempDir::new();
        let root = canonical(&dir);
        fs::write(root.join("Berserk v01.cbz"), b"cbz").unwrap();
        let away = away_from(&root);
        let (watch, scans, _events) = kept_watching(&root);
        tokio::time::sleep(seconds(1)).await;
        assert!(watch.watching(1));

        fs::rename(&root, &away).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("Pluto v01.cbz"), b"cbz").unwrap();
        tokio::time::sleep(RECHECK).await;

        assert!(watch.watching(1));
        assert_eq!(scans.status().pending, by_the_watcher(root.clone()));
        fs::remove_dir_all(&away).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn a_watched_folder_replaced_by_an_empty_one_is_gone_and_not_scanned() {
        let dir = TempDir::new();
        let root = canonical(&dir);
        fs::write(root.join("Berserk v01.cbz"), b"cbz").unwrap();
        let away = away_from(&root);
        let (watch, scans, _events) = kept_watching(&root);
        tokio::time::sleep(seconds(1)).await;
        assert!(watch.watching(1));

        // An unplugged drive's mount point on Linux.
        fs::rename(&root, &away).unwrap();
        fs::create_dir(&root).unwrap();
        tokio::time::sleep(RECHECK).await;

        assert!(!watch.watching(1));
        assert_eq!(lock(&watch.watched)[&1].state, State::Gone);
        assert_eq!(scans.status().pending, None);
        fs::remove_dir_all(&away).unwrap();
    }
}
