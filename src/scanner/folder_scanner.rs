//! The scan: each added directory walked once, each book file and image folder in it stored in a
//! transaction of its own, and what couldn't be read or stored reported.

use std::{
    collections::{BTreeSet, HashSet},
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read},
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use blake3::Hasher;
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{Connection, ToSql};
use tracing::{debug, info, warn};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use crate::book_pages;
use crate::data_models::models::{
    BookDatabaseColumns, BookFormat, BookLanguage, BookMetadata, BookSeriesMetadata, DatabaseTypes,
    Directory, ParsedName, QuerySeparator, SeriesLibraryConnection, WithId,
};
use crate::data_models::scan_model::{
    ScanFailure, ScanProblem, ScanProgress, ScanReport, ScanScope,
};
use crate::db::{checked_unix_time, Database};
use crate::error_types::{AppErrors, DatabaseError};
use crate::scanner::comic_info;
use crate::scanner::cover_image_retriever::{cover_for, Covers, DEFAULT_IMAGE};
use crate::scanner::heartbeat::Heartbeat;

/// How much of a file is hashed between two signs of life: a read crawling at 5 KB/s still beats
/// well within the 2 minutes after which a scan counts as stalled.
const HASH_CHUNK: usize = 256 * 1024;

// Filename patterns, compiled once on first use and then reused.
// A keyword and its number may be joined by nothing, a space, `.`, `-` or — for a keyword of two
// letters or more — `. `: `v01`, `vol 3`, `vol.3`, `v-3`, `Vol. 3`. A lone letter takes no `. `:
// `c. 1920` is circa 1920, not chapter 1920.
static VOLUME_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:v[ .-]?|(?:vol|volume|book)(?:\. |[ .-])?)(\d+)\b").unwrap()
});

// A chapter may be a decimal one: `c01.5` lies between chapters 1 and 2.
static CHAPTER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:c[ .-]?|(?:ch|chap|chapter)(?:\. |[ .-])?)(\d+(?:\.\d+)?)\b").unwrap()
});

static PAGE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:p[ .-]?|(?:pa|page)(?:\. |[ .-])?)(\d+)\b").unwrap());

// Up to three digits: a fourth makes it a year — `Radio-Craft August 1936` — not a volume.
static TRAILING_NUMBER_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s(\d{1,3})\s*$").unwrap());

// Tags in brackets — `[Group]`, `(Digital)`, `{HQ}` — say who made a file, not what it is.
static TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[[^\]]*\]|\([^)]*\)|\{[^}]*\}").unwrap());

// `v`, `c`, `p` and `book` also end ordinary words — `bleach12`, `notebook2` — so only the long
// markers are cut out of a run-together name.
static RUN_TOGETHER_MARKER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:volume|vol|chapter)\d").unwrap());

/// Scans the directories `scope` reads and answers what it did, once its last book is written.
/// `progress` hears how many books the walk found, then how far the scan got after each book.
/// `heartbeat` hears every step; once the worker gives up on the scan, it writes nothing more.
pub fn scan(
    db: &Database,
    covers: &Covers,
    scope: &ScanScope,
    heartbeat: &Heartbeat,
    progress: &mut dyn FnMut(ScanProgress),
) -> Result<ScanReport, AppErrors> {
    let directories: Vec<WithId<Directory>> = db.get_all_entries::<Directory>()?;
    let targets = targets(&directories, scope, heartbeat);
    // Every folder is walked before any book is read, so the progress knows how many there are.
    let walks: Vec<Walk> = targets
        .iter()
        .map(|target| {
            heartbeat.in_directory(target.directory.id, target.root());
            walk(target.root(), &target.folder, heartbeat)
        })
        .collect();
    let found = walks.iter().map(|walk| walk.books.len()).sum();
    progress(ScanProgress {
        found,
        ..ScanProgress::default()
    });
    let context = ScanContext::new(&targets, &walks);
    let mut report = ScanReport::default();
    // The series that moved books left: those left without books go at the end.
    let mut left = BTreeSet::new();
    for (target, walk) in targets.iter().zip(walks) {
        let directory_id = target.directory.id;
        heartbeat.in_directory(directory_id, target.root());
        record(
            &mut report,
            directory_id,
            target.root(),
            walk,
            |book| {
                if heartbeat.given_up() {
                    return Err(ScanProblem::Stalled);
                }
                heartbeat.at(&book.path);
                let stored = store(db, covers, directory_id, book, &context, heartbeat);
                if let Ok(Stored::Moved(Some(series)) | Stored::Updated(Some(series))) = stored {
                    left.insert(series);
                }
                stored
            },
            |report| {
                progress(ScanProgress {
                    found,
                    added: report.added,
                    updated: report.updated,
                    unchanged: report.unchanged,
                    moved: report.moved,
                    failed: report.failures.len(),
                })
            },
        );
    }
    let full = *scope == ScanScope::All;
    heartbeat.beat();
    report.removed = transaction(db, heartbeat, |conn| {
        let mut removed = remove_lost(conn, &context, &mut left)?;
        if full {
            removed += remove_unplaced(conn, &mut left)?;
        }
        remove_emptied(conn, &left)?;
        Ok(removed)
    })?;
    if full {
        match remove_orphan_covers(db, covers, heartbeat) {
            Ok(0) => {}
            Ok(removed) => {
                info!("{removed} cover files no book, series or library named were deleted")
            }
            Err(error) => warn!("the cover folder couldn't be tidied: {error}"),
        }
    }
    Ok(report)
}

/// What the walks of one scan met: what tells the row of a book moved or deleted apart from one
/// still in place.
#[derive(Debug, Default)]
struct ScanContext {
    /// Every book path the walks met, as it is stored.
    seen: HashSet<String>,
    /// The folders a walk couldn't read; the rows under them stay.
    unreadable: Vec<PathBuf>,
    /// The folders the scan read, each with its directory and whether it was available: its root
    /// read, and — for the whole directory — a book found in it. An unplugged drive's empty mount
    /// point is not.
    read: Vec<(i64, PathBuf, bool)>,
}

impl ScanContext {
    fn new(targets: &[Target], walks: &[Walk]) -> ScanContext {
        let mut context = ScanContext::default();
        for (target, walk) in targets.iter().zip(walks) {
            let root_read = !walk.problems.iter().any(|(_, problem)| {
                matches!(
                    problem,
                    ScanProblem::DirectoryMissing | ScanProblem::DirectoryUnreadable
                )
            });
            // A folder inside the directory was there when the scan began, so the drive is too.
            let available = root_read && (!target.whole() || !walk.books.is_empty());
            context
                .read
                .push((target.directory.id, target.folder.clone(), available));
            context.seen.extend(
                walk.books
                    .iter()
                    .map(|book| book.path.to_string_lossy().into_owned()),
            );
            context.unreadable.extend(
                walk.problems
                    .iter()
                    .filter(|(_, problem)| *problem == ScanProblem::FolderUnreadable)
                    .map(|(path, _)| path.clone()),
            );
        }
        context
    }

    /// Whether the book stored at `path` in `directory_id` is gone from where this scan looked: it
    /// lies in a folder read and available, no walk met it, and no folder above it went unread.
    fn lost(&self, directory_id: Option<i64>, path: &str) -> bool {
        let book = Path::new(path);
        directory_id.is_some_and(|id| {
            self.read.iter().any(|(read_id, folder, available)| {
                *read_id == id && *available && book.starts_with(folder)
            })
        }) && !self.seen.contains(path)
            && !self
                .unreadable
                .iter()
                .any(|folder| book.starts_with(folder))
    }

    /// Whether the book stored at `path` in `directory_id` lies in a folder this scan read.
    fn covers(&self, directory_id: Option<i64>, path: &str) -> bool {
        directory_id.is_some_and(|id| {
            self.read
                .iter()
                .any(|(read_id, folder, _)| *read_id == id && Path::new(path).starts_with(folder))
        })
    }
}

/// A folder a scan reads, with the directory it lies in: the directory's own, or one inside it.
struct Target<'a> {
    directory: &'a WithId<Directory>,
    folder: PathBuf,
}

impl Target<'_> {
    fn root(&self) -> &Path {
        Path::new(&self.directory.data.path)
    }

    fn whole(&self) -> bool {
        self.folder == self.root()
    }
}

/// The folders `scope` reads, each with its directory. A folder in no directory is left out, and
/// so is one that is gone, is a symlink or lies behind one, or is spelled in another case.
fn targets<'a>(
    directories: &'a [WithId<Directory>],
    scope: &ScanScope,
    heartbeat: &Heartbeat,
) -> Vec<Target<'a>> {
    match scope {
        ScanScope::All => directories
            .iter()
            .map(|directory| Target {
                directory,
                folder: PathBuf::from(&directory.data.path),
            })
            .collect(),
        ScanScope::Folders(folders) => folders
            .iter()
            .filter_map(|folder| {
                let directory = directories
                    .iter()
                    .find(|directory| folder.starts_with(&directory.data.path))?;
                heartbeat.in_directory(directory.id, Path::new(&directory.data.path));
                heartbeat.at(folder);
                // The root is stored canonical, so a folder inside it reached without a symlink is
                // canonical too; macOS's canonical form also carries each name's case on disk.
                let plain = folder == Path::new(&directory.data.path)
                    || fs::canonicalize(folder).is_ok_and(|real| real == *folder);
                if !plain {
                    debug!("not scanning {folder:?}: gone, or reached through a symlink");
                    return None;
                }
                Some(Target {
                    directory,
                    folder: folder.clone(),
                })
            })
            .collect(),
    }
}

/// Whether the folder at `path` holds anything: an unplugged drive leaves its mount point missing
/// or empty.
pub(crate) fn holds_anything(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}

/// Deletes the rows of the available directories this scan read whose files no walk met; answers
/// how many, and adds their series to `left`.
fn remove_lost(
    conn: &Connection,
    context: &ScanContext,
    left: &mut BTreeSet<i64>,
) -> Result<usize, DatabaseError> {
    let available: BTreeSet<i64> = context
        .read
        .iter()
        .filter(|(_, _, available)| *available)
        .map(|(directory_id, _, _)| *directory_id)
        .collect();
    let mut removed = 0;
    for directory_id in available {
        for (id, path, series) in
            Database::books_placed_in_with_connection(conn, Some(directory_id))?
        {
            if context.lost(Some(directory_id), &path) {
                Database::remove_entry_with_connection(conn, DatabaseTypes::Books, id)?;
                removed += 1;
                left.extend(series);
            }
        }
    }
    Ok(removed)
}

/// Deletes every book in no directory: its file is never served, and a full scan reached every
/// directory. Answers how many, and adds their series to `left`.
fn remove_unplaced(conn: &Connection, left: &mut BTreeSet<i64>) -> Result<usize, DatabaseError> {
    let unplaced = Database::books_placed_in_with_connection(conn, None)?;
    for (id, _, series) in &unplaced {
        Database::remove_entry_with_connection(conn, DatabaseTypes::Books, *id)?;
        left.extend(*series);
    }
    Ok(unplaced.len())
}

/// Deletes the series of `left` that hold no book any more.
fn remove_emptied(conn: &Connection, left: &BTreeSet<i64>) -> Result<(), DatabaseError> {
    for &series_id in left {
        if Database::get_books_in_series_with_connection(conn, series_id)?.is_empty() {
            Database::remove_entry_with_connection(conn, DatabaseTypes::Series, series_id)?;
        }
    }
    Ok(())
}

/// Deletes the files in the cover folder that no book, series or library names, except the
/// fallback cover; answers how many went. It runs inside the scan job, so no other running scan
/// writes a cover meanwhile; one given up on stops before its next deletion.
fn remove_orphan_covers(
    db: &Database,
    covers: &Covers,
    heartbeat: &Heartbeat,
) -> Result<usize, Box<dyn std::error::Error>> {
    let named = db.read_now(|conn| Ok(Database::cover_names_with_connection(conn)?))?;
    let mut removed = 0;
    for entry in fs::read_dir(&covers.folder)? {
        let entry = entry?;
        let kept = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name == DEFAULT_IMAGE || named.contains(name));
        if !kept && entry.file_type()?.is_file() {
            heartbeat.check()?;
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// A book file or an image folder that the walk found.
#[derive(Debug)]
struct Found {
    path: PathBuf,
    format: BookFormat,
}

impl Found {
    /// The name its title, volume, chapter and page are read from: a file's without its
    /// extension, a folder's whole — "Mr. Robot" has none to drop.
    fn name(&self) -> String {
        let name = match self.format {
            BookFormat::ImageComic => self.path.file_name(),
            _ => self.path.file_stem(),
        };
        name.unwrap_or_default().to_string_lossy().into_owned()
    }

    /// Its title and numbers. A name that leaves no title — `Chapter 12`, `v01` — takes the
    /// title of the nearest folder above it that has one.
    fn parsed_name(&self) -> ParsedName {
        let mut parsed = parse_name(&self.name());
        if parsed.title.is_empty() {
            parsed.title = self
                .path
                .ancestors()
                .skip(1)
                .filter_map(Path::file_name)
                .map(|name| parse_name(&name.to_string_lossy()).title)
                .find(|title| !title.is_empty())
                .unwrap_or_default();
        }
        parsed
    }
}

/// What a walk found: the books, and the folders it couldn't read.
#[derive(Debug, Default)]
struct Walk {
    books: Vec<Found>,
    problems: Vec<(PathBuf, ScanProblem)>,
}

/// The book files and image folders in `folder` — the directory at `root`, or a folder inside it —
/// in name order, and what couldn't be read on the way. A symlink is never followed and a hidden
/// entry never read; images lying in `root` itself make no book, while a folder inside it can be
/// one.
fn walk(root: &Path, folder: &Path, heartbeat: &Heartbeat) -> Walk {
    let mut walk = Walk::default();
    // `walkdir` would report this as its first error, without telling a missing folder apart
    // from one the system keeps it out of.
    if let Err(error) = fs::read_dir(root) {
        warn!("the directory {root:?} can't be read: {error}");
        let problem = match error.kind() {
            io::ErrorKind::NotFound => ScanProblem::DirectoryMissing,
            _ => ScanProblem::DirectoryUnreadable,
        };
        walk.problems.push((root.to_path_buf(), problem));
        return walk;
    }
    let entries = WalkDir::new(folder)
        .min_depth(if folder == root { 1 } else { 0 })
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !is_hidden(entry.file_name()));
    for entry in entries {
        let entry = match entry {
            Ok(entry) => {
                heartbeat.at(entry.path());
                entry
            }
            Err(error) => {
                heartbeat.beat();
                warn!("part of the directory {root:?} can't be read: {error}");
                let path = error.path().unwrap_or(root).to_path_buf();
                walk.problems.push((path, ScanProblem::FolderUnreadable));
                continue;
            }
        };
        // The entry's own type — `walkdir` follows no symlink: what one leads to may lie outside
        // the directory, and a book read there would show its page 1 as a cover.
        let kind = entry.file_type();
        let format = if kind.is_dir() {
            book_pages::holds_pages(entry.path()).then_some(BookFormat::ImageComic)
        } else if kind.is_file() {
            book_format(entry.path())
        } else {
            None
        };
        if let Some(format) = format {
            walk.books.push(Found {
                path: entry.into_path(),
                format,
            });
        }
    }
    walk
}

/// A hidden entry, never read: a dot name — macOS's `._` copies and `.Trashes` among them — or
/// the thumbnails a Synology NAS keeps in `@eaDir`.
pub(crate) fn is_hidden(name: &OsStr) -> bool {
    name.as_encoded_bytes().starts_with(b".") || name == "@eaDir"
}

/// The format of a book file, by its extension in any case.
fn book_format(path: &Path) -> Option<BookFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "pdf" => Some(BookFormat::Pdf),
        "epub" => Some(BookFormat::Epub),
        "cbz" => Some(BookFormat::Cbz),
        _ => None,
    }
}

/// How a found book left the scan.
#[derive(Debug, PartialEq)]
enum Stored {
    /// Stored as a new book.
    Added,
    /// Read again — its file changed, or a cover came at last — or its name parses differently
    /// now; with the series it left, if it left one.
    Updated(Option<i64>),
    /// Left as it was, or only its fingerprint renewed.
    Unchanged,
    /// A row whose file this scan lost took it over; with the series that row left, if it left one.
    Moved(Option<i64>),
}

/// Adds the walk's problems to `report`, then reads its books one at a time with `read`, telling
/// `after_each` the report so far. A book that fails — or makes a library reading it panic —
/// costs that book only.
fn record(
    report: &mut ScanReport,
    directory_id: i64,
    root: &Path,
    walk: Walk,
    mut read: impl FnMut(&Found) -> Result<Stored, ScanProblem>,
    mut after_each: impl FnMut(&ScanReport),
) {
    let failure = |path: &Path, problem| ScanFailure::at(directory_id, root, path, problem);
    for (path, problem) in walk.problems {
        report.failures.push(failure(&path, problem));
    }
    after_each(report);
    for found in &walk.books {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| read(found))).unwrap_or_else(|_| {
            warn!("reading {:?} panicked; the scan goes on", found.path);
            Err(ScanProblem::Panicked)
        });
        match outcome {
            Ok(Stored::Added) => report.added += 1,
            Ok(Stored::Updated(_)) => report.updated += 1,
            Ok(Stored::Unchanged) => report.unchanged += 1,
            Ok(Stored::Moved(_)) => report.moved += 1,
            Err(problem) => report.failures.push(failure(&found.path, problem)),
        }
        after_each(report);
    }
}

/// Stores `found` as a new book of the directory — or as a moved one, in the row whose file
/// `context` shows lost — or brings its stored row up to date with it.
fn store(
    db: &Database,
    covers: &Covers,
    directory_id: i64,
    found: &Found,
    context: &ScanContext,
    heartbeat: &Heartbeat,
) -> Result<Stored, ScanProblem> {
    let file_path = found.path.to_string_lossy().into_owned();
    // Taken before the file is read: a change while it is read then shows at the next scan.
    let fingerprint = fingerprint(found);
    let stored = db
        .search_for_single_row_with_id::<BookMetadata>(
            &[BookDatabaseColumns::FilePath],
            &[&file_path],
            QuerySeparator::And,
        )
        .map_err(|_| ScanProblem::StoreFailed)?;
    match stored {
        Some(stored) => refresh(
            db,
            covers,
            directory_id,
            found,
            stored,
            fingerprint,
            heartbeat,
        ),
        None => add(
            db,
            covers,
            directory_id,
            found,
            fingerprint,
            context,
            heartbeat,
        ),
    }
}

/// Stores `found` as a new book of the directory: in the row of a book it is a moved copy of,
/// else in a new one. Its file is read first — hash, cover, reading direction — and one
/// transaction then finds or makes its series and inserts it, so a failure leaves neither behind.
fn add(
    db: &Database,
    covers: &Covers,
    directory_id: i64,
    found: &Found,
    fingerprint: Option<String>,
    context: &ScanContext,
    heartbeat: &Heartbeat,
) -> Result<Stored, ScanProblem> {
    let file_path = found.path.to_string_lossy().into_owned();
    let contents = read_contents(found, heartbeat)?;
    let parsed = found.parsed_name();
    let moved = moved_from(db, &contents.hash, context).map_err(|_| ScanProblem::StoreFailed)?;
    if let Some((id, old_path)) = moved {
        let place = Place {
            directory_id,
            found,
            file_path: &file_path,
            fingerprint: &fingerprint,
        };
        if let Some(stored) = take_over(db, heartbeat, id, &old_path, &place, &contents, &parsed)? {
            return Ok(stored);
        }
    }
    let cover_image = cover_for(covers, &found.path, &found.format, Some(&contents.hash));
    let reading_direction = comic_info::reading_direction(&found.format, &found.path);

    let mut book = BookMetadata {
        title: parsed.title,
        author: "Unknown Author".to_string(),
        format: found.format.clone(),
        tags: vec![],
        description: None,
        file_path,
        language: BookLanguage::Other("Unknown".to_string()),
        page_count: None,
        cover_image,
        series: None,
        volume_number: parsed.volume_number,
        chapter_number: parsed.chapter_number,
        page_number: parsed.page_number,
        file_hash: Some(contents.hash),
        last_modified: contents.modified,
        file_size: contents.size,
        reading_direction,
        directory_id: Some(directory_id),
        fingerprint,
    };
    transaction(db, heartbeat, move |conn| {
        // Another scan may have stored it since the check above.
        let stored = Database::search_for_single_row_with_connection::<BookMetadata>(
            conn,
            &[BookDatabaseColumns::FilePath],
            &[&book.file_path],
            QuerySeparator::And,
        )?;
        if stored.is_some() {
            return Ok(Stored::Unchanged);
        }
        book.series = Some(series_for(conn, &book.title, directory_id)?);
        Database::insert_with_connection(conn, &book)?;
        Ok(Stored::Added)
    })
    .map_err(|_| ScanProblem::StoreFailed)
}

/// The row a new file whose contents hash to `hash` takes over, as `(id, stored path)`: one whose
/// file this scan lost, or — outside the scan — one whose file is gone while its directory is still
/// there, so a copy elsewhere never takes an unplugged drive's book.
fn moved_from(
    db: &Database,
    hash: &str,
    context: &ScanContext,
) -> Result<Option<(i64, String)>, AppErrors> {
    let rows = db.read_now(|conn| Ok(Database::books_with_hash_with_connection(conn, hash)?))?;
    for (id, path, directory_id) in rows {
        let moved = if context.covers(directory_id, &path) {
            context.lost(directory_id, &path)
        } else {
            !Path::new(&path).exists()
                && match directory_id {
                    Some(directory_id) => root_present(db, directory_id)?,
                    None => true,
                }
        };
        if moved {
            return Ok(Some((id, path)));
        }
    }
    Ok(None)
}

/// Whether the directory's folder is there and holds something: a drive unplugged leaves its
/// mount point missing or empty.
fn root_present(db: &Database, directory_id: i64) -> Result<bool, AppErrors> {
    let directory = db.read_now(|conn| {
        Ok(Database::get_entry_with_connection::<Directory>(
            conn,
            directory_id,
        )?)
    })?;
    Ok(holds_anything(Path::new(&directory.path)))
}

/// Where a found book lies: its directory, its file, and the file's fingerprint.
struct Place<'a> {
    directory_id: i64,
    found: &'a Found,
    file_path: &'a str,
    fingerprint: &'a Option<String>,
}

/// Moves the row `id`, stored at `old_path`, to `place`: the same id — so the same reading state —
/// with the new path, directory, name and numbers, in the series its name finds in that directory.
/// `None` when the row changed since it was picked.
fn take_over(
    db: &Database,
    heartbeat: &Heartbeat,
    id: i64,
    old_path: &str,
    place: &Place,
    contents: &Contents,
    parsed: &ParsedName,
) -> Result<Option<Stored>, ScanProblem> {
    transaction(db, heartbeat, |conn| {
        let row = match Database::get_entry_with_connection::<BookMetadata>(conn, id) {
            Ok(row) => row,
            Err(DatabaseError::EntryNotFound) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if row.file_path != old_path {
            return Ok(None);
        }
        let series = series_for(conn, &parsed.title, place.directory_id)?;
        let mut changes = Changes::default();
        changes.set(BookDatabaseColumns::FilePath, place.file_path.to_string());
        changes.set(BookDatabaseColumns::DirectoryId, place.directory_id);
        changes.set(
            BookDatabaseColumns::Format,
            place.found.format.as_str().to_string(),
        );
        changes.set(BookDatabaseColumns::Title, parsed.title.clone());
        changes.set(BookDatabaseColumns::VolumeNumber, parsed.volume_number);
        changes.set(BookDatabaseColumns::ChapterNumber, parsed.chapter_number);
        changes.set(BookDatabaseColumns::PageNumber, parsed.page_number);
        changes.set(BookDatabaseColumns::Series, series);
        changes.set(BookDatabaseColumns::Fingerprint, place.fingerprint.clone());
        changes.set(
            BookDatabaseColumns::LastModified,
            contents.modified.and_then(checked_unix_time),
        );
        changes.apply(conn, id)?;
        Ok(Some(Stored::Moved(row.series.filter(|old| *old != series))))
    })
    .map_err(|_| ScanProblem::StoreFailed)
}

/// Brings the stored book up to date with its file: one whose fingerprint is unchanged isn't read,
/// one only touched takes its new fingerprint and time, and one changed its new contents — under
/// the same id. A book without a cover tries again for one. A name that parses differently than
/// when it was stored — the rules changed since — gives the book its new title and numbers, in the
/// series of `directory_id` that title finds.
fn refresh(
    db: &Database,
    covers: &Covers,
    directory_id: i64,
    found: &Found,
    stored: WithId<BookMetadata>,
    fingerprint: Option<String>,
    heartbeat: &Heartbeat,
) -> Result<Stored, ScanProblem> {
    let book = &stored.data;
    let mut changes = Changes::default();
    let mut updated = false;
    let hash = match &book.file_hash {
        Some(hash) if fingerprint.is_some() && fingerprint == book.fingerprint => hash.clone(),
        _ => {
            let contents = read_contents(found, heartbeat)?;
            changes.set(
                BookDatabaseColumns::LastModified,
                contents.modified.and_then(checked_unix_time),
            );
            changes.set(BookDatabaseColumns::Fingerprint, fingerprint);
            if book.file_hash.as_ref() != Some(&contents.hash) {
                updated = true;
                changes.set(BookDatabaseColumns::FileHash, contents.hash.clone());
                changes.set(
                    BookDatabaseColumns::FileSize,
                    contents.size.map(|size| size as i64),
                );
                changes.set(
                    BookDatabaseColumns::ReadingDirection,
                    comic_info::reading_direction(&found.format, &found.path),
                );
                // Covers are named by the hash: new contents need a cover of their own.
                changes.set(
                    BookDatabaseColumns::CoverImage,
                    cover_for(covers, &found.path, &found.format, Some(&contents.hash)),
                );
            }
            contents.hash
        }
    };
    // A cover can come later: once PDFium is installed, say.
    if book.cover_image.is_none() && !updated {
        if let Some(cover) = cover_for(covers, &found.path, &found.format, Some(&hash)) {
            updated = true;
            changes.set(BookDatabaseColumns::CoverImage, Some(cover));
        }
    }
    let parsed = found.parsed_name();
    let reparsed = parsed.title != book.title
        || parsed.volume_number != book.volume_number
        || parsed.chapter_number != book.chapter_number
        || parsed.page_number != book.page_number;
    if reparsed {
        updated = true;
        changes.set(BookDatabaseColumns::Title, parsed.title.clone());
        changes.set(BookDatabaseColumns::VolumeNumber, parsed.volume_number);
        changes.set(BookDatabaseColumns::ChapterNumber, parsed.chapter_number);
        changes.set(BookDatabaseColumns::PageNumber, parsed.page_number);
    }
    let mut left = None;
    if !changes.columns.is_empty() {
        left = transaction(db, heartbeat, |conn| {
            let mut left = None;
            if reparsed {
                let series = series_for(conn, &parsed.title, directory_id)?;
                changes.set(BookDatabaseColumns::Series, series);
                left = book.series.filter(|old| *old != series);
            }
            changes.apply(conn, stored.id)?;
            Ok(left)
        })
        .map_err(|_| ScanProblem::StoreFailed)?;
    }
    Ok(if updated {
        Stored::Updated(left)
    } else {
        Stored::Unchanged
    })
}

/// A write of the scan: one `IMMEDIATE` transaction that first checks the worker didn't give up on
/// the scan. Checked once the write lock is held, so no write of a scan given up on can come after
/// a write of the next one.
fn transaction<T>(
    db: &Database,
    heartbeat: &Heartbeat,
    operation: impl FnOnce(&Connection) -> Result<T, AppErrors>,
) -> Result<T, AppErrors> {
    db.setup_new_transaction(|conn| {
        heartbeat.check()?;
        operation(conn)
    })
}

/// Columns of a stored book to change, and their new values.
#[derive(Default)]
struct Changes {
    columns: Vec<BookDatabaseColumns>,
    values: Vec<Box<dyn ToSql>>,
}

impl Changes {
    fn set(&mut self, column: BookDatabaseColumns, value: impl ToSql + 'static) {
        self.columns.push(column);
        self.values.push(Box::new(value));
    }

    fn apply(&self, conn: &Connection, id: i64) -> Result<usize, DatabaseError> {
        let values: Vec<&dyn ToSql> = self.values.iter().map(|value| value.as_ref()).collect();
        Database::update_multiple_values_with_connection::<BookMetadata>(
            conn,
            &self.columns,
            &values,
            id,
        )
    }
}

/// What a book's file holds: its BLAKE3 hash, its size and when it last changed. An image
/// folder's hash covers its pages in page order, its size is theirs summed, and its time the
/// newest page's.
struct Contents {
    hash: String,
    size: Option<u64>,
    modified: Option<SystemTime>,
}

fn read_contents(found: &Found, heartbeat: &Heartbeat) -> Result<Contents, ScanProblem> {
    let unreadable = |path: &Path, error: &dyn std::fmt::Display| {
        warn!("{path:?} can't be read: {error}");
        ScanProblem::FileUnreadable
    };
    let files = match found.format {
        BookFormat::ImageComic => book_pages::folder_pages(&found.path)
            .map_err(|error| unreadable(&found.path, &error))?,
        _ => vec![found.path.clone()],
    };
    if files.is_empty() {
        return Err(unreadable(&found.path, &"its pages are gone"));
    }
    let mut hasher = Hasher::new();
    let mut size = Some(0);
    let mut modified = None;
    let mut chunk = vec![0; HASH_CHUNK];
    for path in &files {
        let mut file = File::open(path).map_err(|error| unreadable(path, &error))?;
        let metadata = file.metadata().ok();
        size = size
            .zip(metadata.as_ref().map(|metadata| metadata.len()))
            .map(|(sum, len)| sum + len);
        modified = modified.max(metadata.and_then(|metadata| metadata.modified().ok()));
        loop {
            let read = match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(unreadable(path, &error)),
            };
            hasher.update(&chunk[..read]);
            heartbeat.beat();
        }
    }
    Ok(Contents {
        hash: hasher.finalize().to_hex().to_string(),
        size,
        modified,
    })
}

/// What `found` looks like on disk, told without opening it: a file's size and modification time
/// in nanoseconds — `last_modified` keeps whole seconds, which misses two saves within one — and an
/// image folder's digest of its pages' and its `ComicInfo.xml`'s names, sizes and times. `None`
/// when the system can't tell, which has it read every time.
fn fingerprint(found: &Found) -> Option<String> {
    match found.format {
        BookFormat::ImageComic => folder_fingerprint(&found.path),
        _ => file_fingerprint(&fs::metadata(&found.path).ok()?),
    }
}

fn file_fingerprint(metadata: &fs::Metadata) -> Option<String> {
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("{}:{}", metadata.len(), modified.as_nanos()))
}

fn folder_fingerprint(folder: &Path) -> Option<String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(folder).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let counts = name.to_str().is_some_and(|name| {
            book_pages::page_media_type(name).is_some()
                || name.eq_ignore_ascii_case(comic_info::FILE_NAME)
        });
        if counts && entry.file_type().ok()?.is_file() {
            entries.push((name, file_fingerprint(&entry.metadata().ok()?)?));
        }
    }
    entries.sort();
    let mut hasher = Hasher::new();
    for (name, print) in &entries {
        hasher.update(name.as_encoded_bytes());
        hasher.update(b"\0");
        hasher.update(print.as_bytes());
        hasher.update(b"\n");
    }
    Some(hasher.finalize().to_hex().to_string())
}

/// The key a series is found by: `title` lowercased, then composed — every letter with an accent
/// as one character.
fn match_key(title: &str) -> String {
    // Lowercased in Rust, which knows every alphabet: "ÄRGER v02" finds "Ärger". Composed because
    // older macOS disks store "ä" as "a" and an accent mark — last, as lowercasing may undo it.
    title.to_lowercase().nfc().collect()
}

/// The series a book titled `title` joins: the one whose [`match_key`] is the title's and that holds
/// a book of the same directory, else a new one named as found and put into the directory's
/// library, if it has one. A failing lookup fails the book, as "none" would make a second series.
fn series_for(conn: &Connection, title: &str, directory_id: i64) -> Result<i64, DatabaseError> {
    let key = match_key(title);
    if let Some(id) =
        Database::series_of_directory_by_match_key_with_connection(conn, directory_id, &key)?
    {
        return Ok(id);
    }
    // Read here rather than when the scan started: the library may have been deleted since, and
    // linking to it would fail the book.
    let directory = Database::get_entry_with_connection::<Directory>(conn, directory_id)?;
    let id = Database::insert_with_connection(
        conn,
        &BookSeriesMetadata {
            name: title.to_string(),
            match_key: key,
            description: None,
            cover_image: None,
            start_release_year: None,
            end_release_year: None,
        },
    )?;
    if let Some(library_id) = directory.library_id {
        Database::insert_with_connection(
            conn,
            &SeriesLibraryConnection {
                series_id: id,
                library_id,
            },
        )?;
    }
    Ok(id)
}

/// Splits a name like `[Group] My_Series v02 c13.5` into a clean title plus the volume / chapter /
/// page numbers found in it; bracketed tags leave the title. The title ends at the first marker —
/// in `Berserk v01 The Black Swordsman` what follows names the volume, not the series. A name
/// without one may end in its volume's bare number: `Picture book 2`. A name without any separator
/// is first split where its words meet: `BerserkV02`.
fn parse_name(name: &str) -> ParsedName {
    // `\b` counts `_` as a letter: as a space, it lets `Berserk_v01` show its volume.
    let name = TAG_REGEX.replace_all(name, " ").replace('_', " ");
    // A name with a separator already shows where its words are; only one without any is guessed at.
    let name = if is_run_together(&name) {
        split_run_together(&name)
    } else {
        name
    };
    let number = |regex: &Regex| {
        regex
            .captures(&name)
            .and_then(|captures| captures.get(1))
            .map(|number| number.as_str().to_string())
    };
    let mut volume = number(&VOLUME_REGEX).and_then(|number| number.parse().ok());
    let chapter = number(&CHAPTER_REGEX).and_then(|number| number.parse().ok());
    let page = number(&PAGE_REGEX).and_then(|number| number.parse().ok());

    let first_marker = [&*VOLUME_REGEX, &*CHAPTER_REGEX, &*PAGE_REGEX]
        .into_iter()
        .filter_map(|regex| regex.find(&name))
        .map(|marker| marker.start())
        .min();
    let mut title = first_marker
        .map(|start| clean_title(&name[..start]))
        .unwrap_or_default();
    if title.is_empty() {
        title = clean_title(&PAGE_REGEX.replace_all(
            &CHAPTER_REGEX.replace_all(&VOLUME_REGEX.replace_all(&name, ""), ""),
            "",
        ));
    }
    if first_marker.is_none() {
        if let Some(captures) = TRAILING_NUMBER_REGEX.captures(&name) {
            let before = clean_title(&name[..captures.get(0).map_or(0, |all| all.start())]);
            if !before.is_empty() {
                volume = captures[1].parse().ok();
                title = before;
            }
        }
    }

    ParsedName {
        title,
        volume_number: volume,
        chapter_number: chapter,
        page_number: page,
    }
}

/// Whether `name`, trimmed, holds no white space and no `-`.
fn is_run_together(name: &str) -> bool {
    !name
        .trim()
        .contains(|c: char| c.is_whitespace() || c == '-')
}

/// `name` with a space before each `volume`, `vol` or `chapter` that a number follows, and before
/// every character that [`starts_word`].
fn split_run_together(name: &str) -> String {
    let name = RUN_TOGETHER_MARKER_REGEX.replace_all(name, " $0");
    let mut split = String::with_capacity(name.len() + 8);
    let mut chars = name.chars().peekable();
    let mut before = None;
    while let Some(c) = chars.next() {
        if before.is_some_and(|before| starts_word(before, c, chars.peek().copied())) {
            split.push(' ');
        }
        split.push(c);
        before = Some(c);
    }
    split
}

/// Whether `c`, between `before` and `after`, starts a word of a run-together name: a capital after
/// a lowercase letter (`yB` in `HellboyBPRD`), the last capital of a run when a lowercase letter
/// follows (`DV` in `BPRDVolume`), or a digit next to a letter (`e1`, `0T`).
fn starts_word(before: char, c: char, after: Option<char>) -> bool {
    // Only letters with a case: a CJK name has no capitals to show its words, and stays as written.
    let cased = |c: char| c.is_uppercase() || c.is_lowercase();
    (before.is_lowercase() && c.is_uppercase())
        || (before.is_uppercase() && c.is_uppercase() && after.is_some_and(char::is_lowercase))
        || (cased(before) && c.is_ascii_digit())
        || (before.is_ascii_digit() && cased(c))
}

/// `text` with `-` as a space and every run of white space one space, trimmed.
fn clean_title(text: &str) -> String {
    text.replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::SystemTime;

    use super::*;
    use crate::data_models::models::{LibraryMetadata, LibraryType};
    use crate::error_types::ScanError;
    use crate::test_support::{TempDatabase, TempDir};

    const RIGHT_TO_LEFT: &[u8] = b"<ComicInfo><Manga>YesAndRightToLeft</Manga></ComicInfo>";

    /// [`super::store`] outside a scan: no walk met anything, so no row counts as moved.
    fn store(
        db: &Database,
        covers: &Covers,
        directory_id: i64,
        found: &Found,
    ) -> Result<Stored, ScanProblem> {
        super::store(
            db,
            covers,
            directory_id,
            found,
            &ScanContext::default(),
            &Heartbeat::default(),
        )
    }

    /// The book file at `path`, as the walk finds it.
    fn found(path: &Path) -> Found {
        Found {
            path: path.to_path_buf(),
            format: book_format(path).expect("a book file"),
        }
    }

    /// The image folder at `path`, as the walk finds it.
    fn folder(path: &Path) -> Found {
        Found {
            path: path.to_path_buf(),
            format: BookFormat::ImageComic,
        }
    }

    /// The books `walk` finds under `root`: each one's path relative to it, its parts joined by
    /// `/` on every system, and its format.
    fn books_under(root: &Path) -> Vec<(String, String)> {
        walk(root, root, &Heartbeat::default())
            .books
            .iter()
            .map(|found| {
                (
                    found
                        .path
                        .strip_prefix(root)
                        .unwrap()
                        .iter()
                        .map(|part| part.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                    found.format.as_str().to_string(),
                )
            })
            .collect()
    }

    fn expected(books: &[(&str, &str)]) -> Vec<(String, String)> {
        books
            .iter()
            .map(|(path, format)| (path.to_string(), format.to_string()))
            .collect()
    }

    fn stored_direction(db: &Database, file_path: &Path) -> Option<String> {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT reading_direction FROM books WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| row.get(0),
                )
                .unwrap())
        })
        .unwrap()
    }

    fn covers_in(dir: &TempDir) -> Covers {
        Covers {
            folder: dir.path().join("covers"),
            pdfium: None,
        }
    }

    /// A directory row for `folder` whose new series join `library_id`; answers its id.
    fn add_directory(db: &Database, folder: &Path, library_id: Option<i64>) -> i64 {
        db.insert(&Directory {
            path: folder.to_string_lossy().into_owned(),
            library_id,
            created_at: SystemTime::now(),
        })
        .unwrap()
    }

    fn add_library(db: &Database, name: &str) -> i64 {
        db.insert(&LibraryMetadata {
            name: name.to_string(),
            library_type: LibraryType::Books,
            cover_image: None,
            description: None,
        })
        .unwrap()
    }

    /// Runs one statement, as a change made outside the scanner would.
    fn sql(db: &Database, statement: &str) {
        db.setup_new_transaction(|conn| {
            conn.execute(statement, []).unwrap();
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_walk_finds_book_files_in_any_case_and_each_folder_of_pages_once() {
        let dir = TempDir::new();
        for name in [
            "Berserk v01.PDF",
            "Dune.Epub",
            "Saga v02.CBZ",
            "notes.txt",
            "loose.jpg",
            "Pluto/1.jpg",
            "Pluto/2.png",
            "Pluto/3.WEBP",
            "Series/Vol 1/001.jpg",
            "Series/Vol 2.cbz",
        ] {
            dir.write(name, b"contents");
        }

        assert_eq!(
            books_under(dir.path()),
            expected(&[
                ("Berserk v01.PDF", "pdf"),
                ("Dune.Epub", "epub"),
                ("Pluto", "image_comic"),
                ("Saga v02.CBZ", "cbz"),
                ("Series/Vol 1", "image_comic"),
                ("Series/Vol 2.cbz", "cbz"),
            ]),
            "the root's loose image makes no book, and `Series` holds no page of its own"
        );
    }

    #[test]
    fn a_hidden_entry_is_never_read_but_a_hidden_directory_is_walked() {
        let dir = TempDir::new();
        for name in [
            ".books/Visible v01.cbz",
            ".books/._Visible v01.cbz",
            ".books/.Trashes/Deleted v01.cbz",
            ".books/.thumbnails/1.jpg",
            ".books/@eaDir/Visible v01.cbz/thumb.jpg",
        ] {
            dir.write(name, b"contents");
        }

        assert_eq!(
            books_under(&dir.path().join(".books")),
            expected(&[("Visible v01.cbz", "cbz")])
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_never_followed_so_a_loop_ends() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new();
        let outside = TempDir::new();
        dir.write("books/Kept v01.cbz", b"archive");
        let private = outside.write("Private v01.cbz", b"archive");
        outside.write("Pages/1.jpg", b"page");
        let books = dir.path().join("books");
        symlink(&private, books.join("Leak v01.cbz")).unwrap();
        symlink(outside.path(), books.join("Linked")).unwrap();
        symlink(&books, books.join("Loop")).unwrap();

        assert_eq!(books_under(&books), expected(&[("Kept v01.cbz", "cbz")]));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_or_closed_directory_and_a_closed_folder_are_reported() {
        use std::os::unix::fs::PermissionsExt;
        let set_mode = |path: &Path, mode| {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        };
        let dir = TempDir::new();
        dir.write("closed/Hidden v01.cbz", b"archive");
        for name in ["open/A v01.cbz", "open/Locked/B v01.cbz", "open/Z v01.cbz"] {
            dir.write(name, b"archive");
        }
        let (missing, closed, open) = (
            dir.path().join("gone"),
            dir.path().join("closed"),
            dir.path().join("open"),
        );
        let locked = open.join("Locked");
        set_mode(&closed, 0o000);
        set_mode(&locked, 0o000);

        let walks = [&missing, &closed, &open].map(|root| walk(root, root, &Heartbeat::default()));
        // So that the folder can be deleted again.
        set_mode(&closed, 0o755);
        set_mode(&locked, 0o755);

        let [missing_walk, closed_walk, open_walk] = walks;
        assert!(missing_walk.books.is_empty() && closed_walk.books.is_empty());
        assert_eq!(
            missing_walk.problems,
            [(missing, ScanProblem::DirectoryMissing)]
        );
        assert_eq!(
            closed_walk.problems,
            [(closed, ScanProblem::DirectoryUnreadable)]
        );
        let names: Vec<&Path> = open_walk
            .books
            .iter()
            .map(|found| found.path.strip_prefix(&open).unwrap())
            .collect();
        assert_eq!(names, [Path::new("A v01.cbz"), Path::new("Z v01.cbz")]);
        assert_eq!(
            open_walk.problems,
            [(locked, ScanProblem::FolderUnreadable)]
        );
    }

    #[test]
    fn a_scanned_cbz_keeps_the_direction_its_comic_info_states() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let berserk = dir.write_cbz(
            "Berserk v01.cbz",
            &[
                ("001.jpg", b"not a picture"),
                ("ComicInfo.xml", RIGHT_TO_LEFT),
            ],
        );
        let pluto = dir.write_cbz("Pluto v01.cbz", &[("001.jpg", b"not a picture")]);

        store(&temp.db, &covers, directory, &found(&berserk)).unwrap();
        store(&temp.db, &covers, directory, &found(&pluto)).unwrap();

        assert_eq!(stored_direction(&temp.db, &berserk).as_deref(), Some("rtl"));
        assert_eq!(stored_direction(&temp.db, &pluto), None);
    }

    fn stored_cover(db: &Database, file_path: &Path) -> Option<String> {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT cover_image FROM books WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| row.get(0),
                )
                .unwrap())
        })
        .unwrap()
    }

    #[test]
    fn a_pdf_without_pdfium_is_catalogued_without_a_cover() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let pdf = dir.write("Berserk v01.pdf", b"%PDF-1.7 tiny");

        store(&temp.db, &covers, directory, &found(&pdf)).unwrap();

        assert_eq!(stored_cover(&temp.db, &pdf), None);
    }

    #[test]
    fn an_epub_whose_cover_cant_be_read_is_catalogued_without_one() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let epub = dir.write_cbz(
            "Dune v01.epub",
            &[(
                "META-INF/container.xml",
                b"<container><rootfiles></container>",
            )],
        );

        store(&temp.db, &covers, directory, &found(&epub)).unwrap();

        assert_eq!(stored_cover(&temp.db, &epub), None);
    }

    /// Every series as `(name, match_key)`, by id.
    fn series_rows(db: &Database) -> Vec<(String, String)> {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .prepare("SELECT name, match_key FROM series ORDER BY id")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap())
        })
        .unwrap()
    }

    fn series_of(db: &Database, file_path: &Path) -> Option<i64> {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT series FROM books WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| row.get(0),
                )
                .unwrap())
        })
        .unwrap()
    }

    fn directory_of(db: &Database, file_path: &Path) -> Option<i64> {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT directory_id FROM books WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| row.get(0),
                )
                .unwrap())
        })
        .unwrap()
    }

    /// Every library link as `(library_id, series_id)`, in the order they were made.
    fn links(db: &Database) -> Vec<(i64, i64)> {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .prepare("SELECT library_id, series_id FROM library_elements ORDER BY rowid")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap())
        })
        .unwrap()
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap())
        })
        .unwrap()
    }

    /// Stores each of `names` as a one-page CBZ in `dir`, a file of the directory `directory`;
    /// answers their paths.
    fn scan_books(dir: &TempDir, db: &Database, directory: i64, names: &[&str]) -> Vec<PathBuf> {
        let covers = covers_in(dir);
        names
            .iter()
            .map(|name| {
                let book = dir.write_cbz(name, &[("001.jpg", b"not a picture")]);
                store(db, &covers, directory, &found(&book)).unwrap();
                book
            })
            .collect()
    }

    #[test]
    fn a_series_is_named_as_found_and_a_volume_in_other_capitals_joins_it() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);

        let books = scan_books(
            &dir,
            &temp.db,
            directory,
            &["Berserk v01.cbz", "BERSERK v02.cbz"],
        );

        assert_eq!(
            series_rows(&temp.db),
            [("Berserk".to_string(), "berserk".to_string())]
        );
        assert_eq!(
            series_of(&temp.db, &books[0]),
            series_of(&temp.db, &books[1])
        );
    }

    #[test]
    fn volumes_named_without_separators_join_one_series() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);

        let books = scan_books(
            &dir,
            &temp.db,
            directory,
            &[
                "HellboyBPRDVolume10Title.cbz",
                "HellboyBPRDVolume11Title.cbz",
            ],
        );

        assert_eq!(
            series_rows(&temp.db),
            [("Hellboy BPRD".to_string(), "hellboy bprd".to_string())]
        );
        assert_eq!(
            title_and_volume(&temp.db, &books[0]),
            ("Hellboy BPRD".to_string(), Some(10))
        );
        assert_eq!(
            title_and_volume(&temp.db, &books[1]),
            ("Hellboy BPRD".to_string(), Some(11))
        );
    }

    #[test]
    fn lowercase_volumes_named_without_separators_join_one_series() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);

        let books = scan_books(
            &dir,
            &temp.db,
            directory,
            &[
                "hellboybprdvolume10title.cbz",
                "hellboybprdvolume11title.cbz",
            ],
        );

        assert_eq!(
            series_rows(&temp.db),
            [("hellboybprd".to_string(), "hellboybprd".to_string())]
        );
        assert_eq!(
            title_and_volume(&temp.db, &books[0]),
            ("hellboybprd".to_string(), Some(10))
        );
        assert_eq!(
            title_and_volume(&temp.db, &books[1]),
            ("hellboybprd".to_string(), Some(11))
        );
    }

    #[test]
    fn letters_beyond_ascii_match_in_any_case() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);

        let books = scan_books(
            &dir,
            &temp.db,
            directory,
            &["Ärger v01.cbz", "ÄRGER v02.cbz"],
        );

        assert_eq!(
            series_rows(&temp.db),
            [("Ärger".to_string(), "ärger".to_string())]
        );
        assert_eq!(
            series_of(&temp.db, &books[0]),
            series_of(&temp.db, &books[1])
        );
    }

    #[test]
    fn a_match_key_is_lowercase_and_composed() {
        for title in [
            "\u{c4}rger",
            "A\u{308}rger",
            "a\u{308}rger",
            "\u{c4}RGER",
            "A\u{308}RGER",
        ] {
            assert_eq!(match_key(title), "\u{e4}rger", "{title:?}");
        }
        assert_eq!(match_key("Berserk"), "berserk");
    }

    #[test]
    fn a_name_with_its_accents_stored_apart_joins_the_series_of_one_with_them_together() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);

        // "Ä" as one character, then as "A" and a combining diaeresis, as older macOS disks store it.
        let books = scan_books(
            &dir,
            &temp.db,
            directory,
            &["\u{c4}rger v01.cbz", "A\u{308}rger v02.cbz"],
        );

        assert_eq!(
            series_rows(&temp.db),
            [("\u{c4}rger".to_string(), "\u{e4}rger".to_string())]
        );
        assert_eq!(
            series_of(&temp.db, &books[0]),
            series_of(&temp.db, &books[1])
        );
    }

    #[test]
    fn a_folders_title_is_its_whole_name_and_a_files_drops_its_extension() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        dir.write("Mr. Robot/1.jpg", b"page");
        let file = dir.write_cbz("Dr. Stone v01.cbz", &[("001.jpg", b"not a picture")]);

        store(
            &temp.db,
            &covers,
            directory,
            &folder(&dir.path().join("Mr. Robot")),
        )
        .unwrap();
        store(&temp.db, &covers, directory, &found(&file)).unwrap();

        assert_eq!(
            series_rows(&temp.db),
            [
                ("Mr. Robot".to_string(), "mr. robot".to_string()),
                ("Dr. Stone".to_string(), "dr. stone".to_string())
            ]
        );
    }

    #[test]
    fn a_renamed_series_still_collects_new_volumes() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let first = scan_books(&dir, &temp.db, directory, &["Berserk v01.cbz"]);
        let series_id = series_of(&temp.db, &first[0]).unwrap();
        // As `PATCH /series/{id}` renames it: the name only.
        temp.db
            .setup_new_transaction(|conn| {
                Database::update_multiple_values_with_connection::<BookSeriesMetadata>(
                    conn,
                    &[crate::models::SeriesDatabaseColumns::Name],
                    &[&"Berserk: Deluxe Edition"],
                    series_id,
                )?;
                Ok(())
            })
            .unwrap();

        let second = scan_books(&dir, &temp.db, directory, &["Berserk v02.cbz"]);

        assert_eq!(series_of(&temp.db, &second[0]), Some(series_id));
        assert_eq!(
            series_rows(&temp.db),
            [("Berserk: Deluxe Edition".to_string(), "berserk".to_string())]
        );
    }

    #[test]
    fn a_failing_series_lookup_stores_no_book_and_no_second_series() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        scan_books(&dir, &temp.db, directory, &["Berserk v01.cbz"]);
        // Without its column, every lookup answers an error.
        sql(
            &temp.db,
            "ALTER TABLE series RENAME COLUMN match_key TO lost",
        );
        let second = dir.write_cbz("Berserk v02.cbz", &[("001.jpg", b"not a picture")]);

        let result = store(&temp.db, &covers_in(&dir), directory, &found(&second));

        assert_eq!(result, Err(ScanProblem::StoreFailed));
        assert_eq!(count(&temp.db, "series"), 1);
        assert_eq!(
            count(&temp.db, "books"),
            1,
            "no row, so the next scan tries the file again"
        );
    }

    #[test]
    fn a_book_whose_insert_fails_leaves_no_series_behind() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        sql(
            &temp.db,
            "CREATE TRIGGER refuse_books BEFORE INSERT ON books \
             BEGIN SELECT RAISE(ABORT, 'refused'); END",
        );
        let book = dir.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);

        let result = store(&temp.db, &covers_in(&dir), directory, &found(&book));

        assert_eq!(result, Err(ScanProblem::StoreFailed));
        assert_eq!(count(&temp.db, "series"), 0, "made for it, gone with it");
    }

    #[cfg(unix)]
    #[test]
    fn a_stored_book_is_not_read_again_and_one_that_cannot_be_read_gets_no_row() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let stored = dir.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);
        let closed = dir.write_cbz("Monster v01.cbz", &[("001.jpg", b"not a picture")]);
        assert_eq!(
            store(&temp.db, &covers, directory, &found(&stored)),
            Ok(Stored::Added)
        );
        for file in [&stored, &closed] {
            fs::set_permissions(file, fs::Permissions::from_mode(0o000)).unwrap();
        }

        assert_eq!(
            store(&temp.db, &covers, directory, &found(&stored)),
            Ok(Stored::Unchanged)
        );
        assert_eq!(
            store(&temp.db, &covers, directory, &found(&closed)),
            Err(ScanProblem::FileUnreadable)
        );
        assert_eq!(count(&temp.db, "books"), 1);
        assert_eq!(count(&temp.db, "series"), 1, "none for the closed book");
    }

    #[test]
    fn a_book_that_panics_is_reported_and_the_next_one_is_stored() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        dir.write_cbz("A v01.cbz", &[("001.jpg", b"not a picture")]);
        let kept = dir.write_cbz("B v01.cbz", &[("001.jpg", b"not a picture")]);
        let mut report = ScanReport::default();

        record(
            &mut report,
            directory,
            dir.path(),
            walk(dir.path(), dir.path(), &Heartbeat::default()),
            |found| {
                if found.path.ends_with("A v01.cbz") {
                    panic!("a library broke on this file");
                }
                store(&temp.db, &covers, directory, found)
            },
            |_| {},
        );

        assert_eq!(
            report,
            ScanReport {
                added: 1,
                updated: 0,
                unchanged: 0,
                moved: 0,
                removed: 0,
                failures: vec![ScanFailure {
                    directory_id: directory,
                    relative_path: "A v01.cbz".to_string(),
                    error_key: ScanProblem::Panicked,
                }],
            }
        );
        assert!(series_of(&temp.db, &kept).is_some());
    }

    #[test]
    fn a_scan_reads_its_scope_tells_its_progress_and_has_stored_its_books_when_it_returns() {
        let books = TempDir::new();
        let others = TempDir::new();
        let elsewhere = TempDir::new();
        let cover_folder = TempDir::new();
        let temp = TempDatabase::open();
        let cbz = books.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);
        books.write("Pluto/1.jpg", b"page");
        others.write_cbz("Saga v01.cbz", &[("001.jpg", b"not a picture")]);
        let present = add_directory(&temp.db, books.path(), None);
        let missing = add_directory(&temp.db, &elsewhere.path().join("gone"), None);
        add_directory(&temp.db, others.path(), None);
        let covers = covers_in(&cover_folder);
        let gone = ScanFailure {
            directory_id: missing,
            relative_path: String::new(),
            error_key: ScanProblem::DirectoryMissing,
        };
        let mut heard = Vec::new();

        let first = scan(
            &temp.db,
            &covers,
            &ScanScope::Folders(BTreeSet::from([
                books.path().to_path_buf(),
                elsewhere.path().join("gone"),
            ])),
            &Heartbeat::default(),
            &mut |progress| heard.push(progress),
        )
        .unwrap();

        assert_eq!(
            first,
            ScanReport {
                added: 2,
                updated: 0,
                unchanged: 0,
                moved: 0,
                removed: 0,
                failures: vec![gone.clone()],
            }
        );
        assert_eq!(
            (heard.first(), heard.last()),
            (
                Some(&ScanProgress {
                    found: 2,
                    ..ScanProgress::default()
                }),
                Some(&ScanProgress {
                    found: 2,
                    added: 2,
                    updated: 0,
                    unchanged: 0,
                    moved: 0,
                    failed: 1,
                })
            )
        );
        assert_eq!(directory_of(&temp.db, &cbz), Some(present));
        assert_eq!(
            directory_of(&temp.db, &books.path().join("Pluto")),
            Some(present)
        );
        assert_eq!(
            count(&temp.db, "books"),
            2,
            "the third directory was not in scope"
        );

        let second = scan(
            &temp.db,
            &covers,
            &ScanScope::All,
            &Heartbeat::default(),
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(
            second,
            ScanReport {
                added: 1,
                updated: 0,
                unchanged: 2,
                moved: 0,
                removed: 0,
                failures: vec![gone],
            }
        );
    }

    #[test]
    fn a_scanned_cbz_stores_its_cover_by_name_from_the_given_folder() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let page = crate::test_support::encoded_image(image::ImageFormat::Png, 2, 3);
        let cbz = dir.write_cbz("Berserk v01.cbz", &[("001.png", &page)]);

        store(&temp.db, &covers, directory, &found(&cbz)).unwrap();

        let name = stored_cover(&temp.db, &cbz).expect("a cover");
        assert!(!name.contains('/'), "a bare file name: {name}");
        assert_eq!(std::fs::read(covers.path_of(&name)).unwrap(), page);
    }

    #[test]
    fn a_book_belongs_to_its_directory_and_its_name_in_another_is_another_series() {
        let manga_folder = TempDir::new();
        let comics_folder = TempDir::new();
        let temp = TempDatabase::open();
        let manga = add_directory(&temp.db, manga_folder.path(), None);
        let comics = add_directory(&temp.db, comics_folder.path(), None);

        let in_manga = scan_books(
            &manga_folder,
            &temp.db,
            manga,
            &["Berserk v01.cbz", "Berserk v02.cbz"],
        );
        let in_comics = scan_books(&comics_folder, &temp.db, comics, &["Berserk v01.cbz"]);

        assert_eq!(directory_of(&temp.db, &in_manga[1]), Some(manga));
        assert_eq!(directory_of(&temp.db, &in_comics[0]), Some(comics));
        assert_eq!(
            series_of(&temp.db, &in_manga[0]),
            series_of(&temp.db, &in_manga[1])
        );
        assert_ne!(
            series_of(&temp.db, &in_manga[0]),
            series_of(&temp.db, &in_comics[0])
        );
        let berserk = ("Berserk".to_string(), "berserk".to_string());
        assert_eq!(series_rows(&temp.db), [berserk.clone(), berserk]);
        assert!(
            links(&temp.db).is_empty(),
            "neither directory has a library"
        );
    }

    #[test]
    fn a_new_series_joins_its_directorys_library_once_and_one_taken_out_stays_out() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let library = add_library(&temp.db, "Manga");
        let directory = add_directory(&temp.db, dir.path(), Some(library));

        let first = scan_books(
            &dir,
            &temp.db,
            directory,
            &["Berserk v01.cbz", "Berserk v02.cbz"],
        );
        let berserk = series_of(&temp.db, &first[0]).unwrap();
        assert_eq!(links(&temp.db), [(library, berserk)]);

        // As `DELETE /libraries/{id}/series/{series_id}` takes it out.
        sql(&temp.db, "DELETE FROM library_elements");
        let later = scan_books(
            &dir,
            &temp.db,
            directory,
            &["Berserk v03.cbz", "Monster v01.cbz"],
        );

        assert_eq!(series_of(&temp.db, &later[0]), Some(berserk));
        let monster = series_of(&temp.db, &later[1]).unwrap();
        assert_eq!(links(&temp.db), [(library, monster)], "only the new series");
    }

    #[test]
    fn a_series_found_after_its_directorys_library_was_deleted_joins_none() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let library = add_library(&temp.db, "Manga");
        let directory = add_directory(&temp.db, dir.path(), Some(library));
        // As `DELETE /libraries/{id}` does: the directory stays, without its library.
        sql(
            &temp.db,
            &format!("DELETE FROM library WHERE id = {library}"),
        );

        let books = scan_books(&dir, &temp.db, directory, &["Monster v01.cbz"]);

        assert!(series_of(&temp.db, &books[0]).is_some());
        assert!(links(&temp.db).is_empty());
    }

    #[test]
    fn a_file_of_a_directory_removed_meanwhile_makes_no_series_and_no_book() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        // As `DELETE /directories/{id}` does while a scan of it still runs.
        sql(&temp.db, "DELETE FROM directories");
        let book = dir.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);

        let result = store(&temp.db, &covers_in(&dir), directory, &found(&book));

        assert_eq!(result, Err(ScanProblem::StoreFailed));
        assert_eq!(count(&temp.db, "series"), 0);
        assert_eq!(count(&temp.db, "books"), 0);
    }

    /// A stored book's id, hash, fingerprint and time, read with plain SQL.
    fn stored_row(db: &Database, file_path: &Path) -> (i64, String, Option<String>, Option<i64>) {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id, file_hash, fingerprint, last_modified FROM books \
                     WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap())
        })
        .unwrap()
    }

    /// Sets the file's modification time to `seconds` after 2000.
    fn set_time(path: &Path, seconds: u64) {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(946_684_800 + seconds);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        crate::test_support::encoded_image(image::ImageFormat::Png, width, height)
    }

    #[cfg(unix)]
    #[test]
    fn an_unchanged_book_with_a_cover_is_not_opened_again() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let cbz = dir.write_cbz("Berserk v01.cbz", &[("001.png", &png(2, 3))]);
        assert_eq!(
            store(&temp.db, &covers, directory, &found(&cbz)),
            Ok(Stored::Added)
        );
        assert!(stored_cover(&temp.db, &cbz).is_some());
        // Its fingerprint stays: a mode change touches neither size nor time.
        fs::set_permissions(&cbz, fs::Permissions::from_mode(0o000)).unwrap();

        let again = store(&temp.db, &covers, directory, &found(&cbz));

        assert_eq!(again, Ok(Stored::Unchanged));
    }

    #[test]
    fn a_touched_book_only_takes_its_new_fingerprint_and_time() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let cbz = dir.write_cbz("Berserk v01.cbz", &[("001.png", &png(2, 3))]);
        set_time(&cbz, 100);
        store(&temp.db, &covers, directory, &found(&cbz)).unwrap();
        let (id, hash, fingerprint, time) = stored_row(&temp.db, &cbz);

        set_time(&cbz, 200);
        let again = store(&temp.db, &covers, directory, &found(&cbz));

        assert_eq!(again, Ok(Stored::Unchanged), "the same contents");
        let (id_after, hash_after, fingerprint_after, time_after) = stored_row(&temp.db, &cbz);
        assert_eq!((id_after, hash_after), (id, hash));
        assert_ne!(fingerprint_after, fingerprint);
        assert_eq!(time_after, time.map(|time| time + 100));
    }

    #[test]
    fn a_changed_book_is_read_again_under_its_id_and_in_its_series() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let cbz = dir.write_cbz("Berserk v01.cbz", &[("001.png", &png(2, 3))]);
        store(&temp.db, &covers, directory, &found(&cbz)).unwrap();
        let (id, hash, _, _) = stored_row(&temp.db, &cbz);
        let (series, cover) = (series_of(&temp.db, &cbz), stored_cover(&temp.db, &cbz));

        let page = png(4, 5);
        dir.write_cbz("Berserk v01.cbz", &[("001.png", &page)]);
        let again = store(&temp.db, &covers, directory, &found(&cbz));

        assert_eq!(again, Ok(Stored::Updated(None)));
        let (id_after, hash_after, _, _) = stored_row(&temp.db, &cbz);
        assert_eq!(id_after, id);
        assert_ne!(hash_after, hash);
        assert_eq!(series_of(&temp.db, &cbz), series);
        let cover_after = stored_cover(&temp.db, &cbz).expect("a cover");
        assert_ne!(Some(&cover_after), cover.as_ref());
        assert_eq!(fs::read(covers.path_of(&cover_after)).unwrap(), page);
        assert_eq!(count(&temp.db, "books"), 1);
    }

    #[test]
    fn a_page_added_to_an_image_folder_reads_the_folder_again() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let first = png(2, 3);
        dir.write("Pluto/1.png", &first);
        let pluto = dir.path().join("Pluto");
        store(&temp.db, &covers, directory, &folder(&pluto)).unwrap();
        let (id, hash, _, _) = stored_row(&temp.db, &pluto);
        assert_eq!(hash, blake3::hash(&first).to_hex().to_string());

        let second = png(4, 5);
        dir.write("Pluto/2.png", &second);
        let again = store(&temp.db, &covers, directory, &folder(&pluto));

        assert_eq!(again, Ok(Stored::Updated(None)));
        let (id_after, hash_after, _, _) = stored_row(&temp.db, &pluto);
        assert_eq!(id_after, id);
        let mut both = blake3::Hasher::new();
        both.update(&first);
        both.update(&second);
        assert_eq!(
            hash_after,
            both.finalize().to_hex().to_string(),
            "in page order"
        );
        let size: i64 = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .query_row("SELECT file_size FROM books", [], |row| row.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(size as usize, first.len() + second.len());
    }

    #[cfg(unix)]
    #[test]
    fn a_book_stored_before_fingerprints_is_read_once_more() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let page = png(2, 3);
        let cbz = dir.write_cbz("Berserk v01.cbz", &[("001.png", &page)]);
        store(&temp.db, &covers, directory, &found(&cbz)).unwrap();
        // As migration 0007 leaves a book: no fingerprint yet.
        sql(&temp.db, "UPDATE books SET fingerprint = NULL");

        let first = store(&temp.db, &covers, directory, &found(&cbz));
        fs::set_permissions(&cbz, fs::Permissions::from_mode(0o000)).unwrap();
        let second = store(&temp.db, &covers, directory, &found(&cbz));

        assert_eq!(first, Ok(Stored::Unchanged), "read, and found the same");
        assert!(stored_row(&temp.db, &cbz).2.is_some());
        assert_eq!(second, Ok(Stored::Unchanged), "and not read again");
    }

    #[test]
    fn a_book_without_a_cover_takes_one_as_soon_as_there_is_one() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let cbz = dir.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);
        store(&temp.db, &covers, directory, &found(&cbz)).unwrap();
        assert_eq!(stored_cover(&temp.db, &cbz), None);
        assert_eq!(
            store(&temp.db, &covers, directory, &found(&cbz)),
            Ok(Stored::Unchanged),
            "still none to take"
        );

        // As if extracted by a newer server, or by PDFium installed since.
        let hash = stored_row(&temp.db, &cbz).1;
        fs::create_dir_all(&covers.folder).unwrap();
        fs::write(covers.path_of(&format!("{hash}.png")), png(2, 3)).unwrap();
        let again = store(&temp.db, &covers, directory, &found(&cbz));

        assert_eq!(again, Ok(Stored::Updated(None)));
        assert_eq!(stored_cover(&temp.db, &cbz), Some(format!("{hash}.png")));
    }

    #[test]
    fn an_image_folders_fingerprint_follows_its_pages_and_its_comic_info_only() {
        let dir = TempDir::new();
        dir.write("Pluto/1.png", b"one");
        dir.write("Pluto/ComicInfo.xml", b"<ComicInfo/>");
        let pluto = dir.path().join("Pluto");
        let print = || folder_fingerprint(&pluto).unwrap();
        let first = print();

        dir.write("Pluto/notes.txt", b"not a page");
        dir.write("Pluto/Extras/1.png", b"a book of its own");
        dir.write("Pluto/.hidden.png", b"hidden");
        assert_eq!(print(), first, "nothing that counts changed");

        for (change, path) in [
            ("a page added", "Pluto/2.png"),
            ("ComicInfo.xml rewritten", "Pluto/ComicInfo.xml"),
        ] {
            let before = print();
            dir.write(path, b"something else entirely");
            assert_ne!(print(), before, "{change}");
        }
        let before = print();
        set_time(&pluto.join("1.png"), 300);
        assert_ne!(print(), before, "a page touched");
    }

    #[test]
    fn a_file_name_gives_its_title_volume_chapter_and_page() {
        for (name, title, volume, chapter, page) in [
            ("Berserk v01", "Berserk", Some(1), None, None),
            ("Berserk vol.3", "Berserk", Some(3), None, None),
            ("Berserk v-3", "Berserk", Some(3), None, None),
            ("Berserk Volume 12", "Berserk", Some(12), None, None),
            ("Berserk_v01_c003", "Berserk", Some(1), Some(3.0), None),
            ("One Piece c01.5", "One Piece", None, Some(1.5), None),
            ("One Piece c.12", "One Piece", None, Some(12.0), None),
            ("One Piece Ch.1001", "One Piece", None, Some(1001.0), None),
            ("One Piece Chapter 7", "One Piece", None, Some(7.0), None),
            (
                "My_Series v02 c13 p4",
                "My Series",
                Some(2),
                Some(13.0),
                Some(4),
            ),
            (
                "[Group] Berserk (Digital) v02 {HQ}",
                "Berserk",
                Some(2),
                None,
                None,
            ),
            ("Mr. Robot", "Mr. Robot", None, None, None),
            ("Chapter 12", "", None, Some(12.0), None),
            // `Book` marks a volume too, as long as a number follows it.
            ("Harry Potter Book 3", "Harry Potter", Some(3), None, None),
            ("Harry Potter book.3", "Harry Potter", Some(3), None, None),
            ("Harry Potter Book3", "Harry Potter", Some(3), None, None),
            ("The Book Thief", "The Book Thief", None, None, None),
            ("Notebook 3", "Notebook", Some(3), None, None),
            ("Picture book 2", "Picture", Some(2), None, None),
            // The title ends at the first marker.
            (
                "Berserk v01 The Black Swordsman",
                "Berserk",
                Some(1),
                None,
                None,
            ),
            (
                "Harry Potter Book 3 The Prisoner of Azkaban",
                "Harry Potter",
                Some(3),
                None,
                None,
            ),
            (
                "One Piece c001 Romance Dawn",
                "One Piece",
                None,
                Some(1.0),
                None,
            ),
            ("Berserk - v01 - Guts", "Berserk", Some(1), None, None),
            ("Berserk p4 cover", "Berserk", None, None, Some(4)),
            ("v01 Berserk", "Berserk", Some(1), None, None),
            // Without a marker, a bare number of up to three digits at the end is the volume.
            ("Picture Book 2", "Picture", Some(2), None, None),
            ("Picture album 2", "Picture album", Some(2), None, None),
            ("Berserk 001", "Berserk", Some(1), None, None),
            ("Berserk - 01", "Berserk", Some(1), None, None),
            (
                "Vinland Saga 07 (Digital)",
                "Vinland Saga",
                Some(7),
                None,
                None,
            ),
            ("Catch 22", "Catch", Some(22), None, None),
            (
                "Radio-Craft August 1936",
                "Radio Craft August 1936",
                None,
                None,
                None,
            ),
            ("1984", "1984", None, None, None),
            ("[Group] 12", "12", None, None, None),
            ("Version 2.0", "Version 2.0", None, None, None),
            ("COVID-19", "COVID 19", None, None, None),
            ("Berserk v01 extra 3", "Berserk", Some(1), None, None),
            // A name without any separator is split where its words meet.
            (
                "HellboyBPRDVolume10Title",
                "Hellboy BPRD",
                Some(10),
                None,
                None,
            ),
            ("BerserkV01", "Berserk", Some(1), None, None),
            ("Berserk01", "Berserk", Some(1), None, None),
            ("OnePieceChapter7", "One Piece", None, Some(7.0), None),
            (
                "[Group]HellboyBPRDVol2(Digital)",
                "Hellboy BPRD",
                Some(2),
                None,
                None,
            ),
            ("ÄrgerV2", "Ärger", Some(2), None, None),
            ("XMen", "X Men", None, None, None),
            ("ONEPIECE", "ONEPIECE", None, None, None),
            ("Berserk", "Berserk", None, None, None),
            ("進撃の巨人1", "進撃の巨人1", None, None, None),
            // One separator, and the name is left as written.
            ("Hellboy BPRDVol2", "Hellboy BPRDVol2", None, None, None),
            // In one without capitals, only a long marker followed by a number is cut out.
            (
                "hellboybprdvolume10title",
                "hellboybprd",
                Some(10),
                None,
                None,
            ),
            ("hellboyvol3", "hellboy", Some(3), None, None),
            ("onepiecechapter12", "onepiece", None, Some(12.0), None),
            ("HELLBOYVOLUME10", "HELLBOY", Some(10), None, None),
            ("bleach12", "bleach", Some(12), None, None),
            ("notebook2", "notebook", Some(2), None, None),
            ("revolver", "revolver", None, None, None),
            // A keyword of two letters or more may take a dot and a space; a lone letter can't.
            (
                "Hellboy Vol. 1 Seed of Destruction",
                "Hellboy",
                Some(1),
                None,
                None,
            ),
            ("Hellboy Vol. 1", "Hellboy", Some(1), None, None),
            ("Berserk Volume. 2", "Berserk", Some(2), None, None),
            ("One Piece Ch. 5", "One Piece", None, Some(5.0), None),
            ("One Piece Chapter. 5.5", "One Piece", None, Some(5.5), None),
            ("Berserk Page. 3", "Berserk", None, None, Some(3)),
            ("London c. 1850", "London c. 1850", None, None, None),
        ] {
            let parsed = parse_name(name);
            assert_eq!(
                (
                    parsed.title.as_str(),
                    parsed.volume_number,
                    parsed.chapter_number,
                    parsed.page_number
                ),
                (title, volume, chapter, page),
                "{name}"
            );
        }
    }

    #[test]
    fn a_name_without_a_title_takes_the_nearest_folder_that_has_one() {
        let parsed = |path: &str, format| {
            Found {
                path: PathBuf::from(path),
                format,
            }
            .parsed_name()
        };

        let file = parsed("/books/Berserk/Chapter 12.cbz", BookFormat::Cbz);
        let nested = parsed("/books/Berserk/Vol 1/c013.cbz", BookFormat::Cbz);
        let pages = parsed("/books/Dune/v01", BookFormat::ImageComic);
        let numbered = parsed("/books/Berserk 2/c013.cbz", BookFormat::Cbz);
        let run_together = parsed("/books/HellboyBPRD/v01", BookFormat::ImageComic);

        assert_eq!(
            (file.title.as_str(), file.chapter_number),
            ("Berserk", Some(12.0))
        );
        assert_eq!(
            (nested.title.as_str(), nested.chapter_number),
            ("Berserk", Some(13.0)),
            "`Vol 1` names no title either"
        );
        assert_eq!(
            (pages.title.as_str(), pages.volume_number),
            ("Dune", Some(1))
        );
        assert_eq!(
            (numbered.title.as_str(), numbered.chapter_number),
            ("Berserk", Some(13.0)),
            "a folder's own number leaves its title"
        );
        assert_eq!(
            (run_together.title.as_str(), run_together.volume_number),
            ("Hellboy BPRD", Some(1)),
            "a folder's name is split as a file's is"
        );
    }

    #[test]
    fn a_book_named_by_its_chapter_alone_joins_its_folders_series() {
        let dir = TempDir::new();
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let book = dir.write_cbz("Berserk/Chapter 12.cbz", &[("001.jpg", b"not a picture")]);

        store(&temp.db, &covers_in(&dir), directory, &found(&book)).unwrap();

        assert_eq!(
            series_rows(&temp.db),
            [("Berserk".to_string(), "berserk".to_string())]
        );
    }

    fn scan_all(db: &Database, covers: &Covers) -> ScanReport {
        scan(
            db,
            covers,
            &ScanScope::All,
            &Heartbeat::default(),
            &mut |_| {},
        )
        .unwrap()
    }

    /// The id of the book stored at `file_path`, if one is.
    fn row_id(db: &Database, file_path: &Path) -> Option<i64> {
        use rusqlite::OptionalExtension;
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM books WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| row.get(0),
                )
                .optional()
                .unwrap())
        })
        .unwrap()
    }

    /// The title and volume stored for the book at `file_path`.
    fn title_and_volume(db: &Database, file_path: &Path) -> (String, Option<i64>) {
        db.setup_new_transaction(|conn| {
            Ok(conn
                .query_row(
                    "SELECT title, volume_number FROM books WHERE file_path = ?1",
                    [file_path.to_string_lossy()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap())
        })
        .unwrap()
    }

    /// A one-page CBZ at `name` in `dir`, its contents its own: no other file hashes alike.
    fn own_cbz(dir: &TempDir, name: &str) -> PathBuf {
        dir.write_cbz(name, &[("001.jpg", name.as_bytes())])
    }

    /// (added, moved, removed)
    fn changes_of(report: &ScanReport) -> (usize, usize, usize) {
        (report.added, report.moved, report.removed)
    }

    #[test]
    fn a_deleted_file_leaves_the_catalog_with_the_series_it_empties() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let berserk = own_cbz(&dir, "Berserk v01.cbz");
        let monster = own_cbz(&dir, "Monster v01.cbz");
        scan_all(&temp.db, &covers);

        fs::remove_file(&monster).unwrap();
        let report = scan_all(&temp.db, &covers);

        assert_eq!(changes_of(&report), (0, 0, 1));
        assert_eq!(row_id(&temp.db, &monster), None);
        assert!(row_id(&temp.db, &berserk).is_some());
        assert_eq!(
            series_rows(&temp.db),
            [("Berserk".to_string(), "berserk".to_string())]
        );
    }

    #[test]
    fn a_renamed_or_moved_file_keeps_its_row() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let first = own_cbz(&dir, "Berserk v01.cbz");
        own_cbz(&dir, "Berserk v02.cbz");
        scan_all(&temp.db, &covers);
        let id = row_id(&temp.db, &first).unwrap();
        let series = series_of(&temp.db, &first);

        let renamed = dir.path().join("Old").join("Berserk_v01.cbz");
        fs::create_dir_all(renamed.parent().unwrap()).unwrap();
        fs::rename(&first, &renamed).unwrap();
        let report = scan_all(&temp.db, &covers);

        assert_eq!(changes_of(&report), (0, 1, 0));
        assert_eq!(row_id(&temp.db, &renamed), Some(id), "the same row");
        assert_eq!(series_of(&temp.db, &renamed), series);
        assert_eq!(count(&temp.db, "books"), 2);
    }

    #[test]
    fn a_book_stored_under_older_name_rules_moves_to_the_series_its_name_finds_now() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let v01 = own_cbz(&dir, "Berserk v01 The Black Swordsman.cbz");
        let v02 = own_cbz(&dir, "Berserk v02 The Golden Age.cbz");
        let v03 = own_cbz(&dir, "Berserk v03 Conviction.cbz");
        let other = own_cbz(&dir, "Berserk The Golden Age.cbz");
        let books = [&v01, &v02, &v03, &other];
        scan_all(&temp.db, &covers);
        let ids = books.map(|book| row_id(&temp.db, book));
        // As the rules before a marker ended the title stored them: v02 in the series of the
        // book titled like it, v03 in one of its own.
        sql(
            &temp.db,
            "UPDATE books SET title = 'Berserk The Golden Age', series = \
             (SELECT id FROM series WHERE match_key = 'berserk the golden age') \
             WHERE file_path LIKE '%v02%'",
        );
        sql(
            &temp.db,
            "INSERT INTO series (name, match_key) VALUES ('Berserk Conviction', 'berserk conviction')",
        );
        sql(
            &temp.db,
            "UPDATE books SET title = 'Berserk Conviction', series = \
             (SELECT id FROM series WHERE match_key = 'berserk conviction') \
             WHERE file_path LIKE '%v03%'",
        );
        // An admin's new name keeps the series' match key.
        sql(
            &temp.db,
            "UPDATE series SET name = 'Berserk (Deluxe)' WHERE match_key = 'berserk'",
        );

        let report = scan_all(&temp.db, &covers);

        assert_eq!((report.updated, report.unchanged), (2, 2));
        assert_eq!(
            books.map(|book| row_id(&temp.db, book)),
            ids,
            "the same rows"
        );
        assert_eq!(
            title_and_volume(&temp.db, &v03),
            ("Berserk".to_string(), Some(3))
        );
        let berserk = series_of(&temp.db, &v01);
        assert_eq!(series_of(&temp.db, &v02), berserk);
        assert_eq!(series_of(&temp.db, &v03), berserk);
        assert_ne!(series_of(&temp.db, &other), berserk);
        assert_eq!(
            series_rows(&temp.db).into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                ("Berserk (Deluxe)".to_string(), "berserk".to_string()),
                (
                    "Berserk The Golden Age".to_string(),
                    "berserk the golden age".to_string()
                ),
            ]),
            "the series v03 emptied is gone, the one still holding a book stays"
        );
    }

    #[test]
    fn a_book_whose_name_parses_as_it_was_stored_is_left_as_it_is() {
        let dir = TempDir::new();
        let covers = covers_in(&dir);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        // A decimal chapter, a whole one SQLite stores as an integer, a page and a bare volume.
        for name in [
            "One Piece c01.5.cbz",
            "One Piece c12.cbz",
            "Berserk v01 p4.cbz",
            "Picture album 2.cbz",
        ] {
            let book = dir.write_cbz(name, &[("001.jpg", b"not a picture")]);
            store(&temp.db, &covers, directory, &found(&book)).unwrap();

            let again = store(&temp.db, &covers, directory, &found(&book));

            assert_eq!(again, Ok(Stored::Unchanged), "{name}");
        }
    }

    #[test]
    fn a_file_renamed_in_its_letter_case_only_keeps_its_row() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let book = own_cbz(&dir, "Berserk v01.cbz");
        scan_all(&temp.db, &covers);
        let id = row_id(&temp.db, &book).unwrap();

        let lower = dir.path().join("berserk v01.cbz");
        fs::rename(&book, &lower).unwrap();
        let report = scan_all(&temp.db, &covers);

        assert_eq!(changes_of(&report), (0, 1, 0));
        assert_eq!(row_id(&temp.db, &lower), Some(id));
    }

    #[test]
    fn a_copy_is_a_book_of_its_own() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let book = own_cbz(&dir, "Berserk v01.cbz");
        scan_all(&temp.db, &covers);
        let id = row_id(&temp.db, &book).unwrap();

        let copy = dir.path().join("Copy").join("Berserk v01.cbz");
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        fs::copy(&book, &copy).unwrap();
        let report = scan_all(&temp.db, &covers);

        assert_eq!(changes_of(&report), (1, 0, 0));
        assert_eq!(row_id(&temp.db, &book), Some(id));
        assert!(row_id(&temp.db, &copy).is_some());
    }

    #[test]
    fn a_book_moved_to_another_directory_takes_a_series_there() {
        let manga = TempDir::new();
        let comics = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, manga.path(), None);
        let library = add_library(&temp.db, "Comics");
        let to = add_directory(&temp.db, comics.path(), Some(library));
        let book = own_cbz(&manga, "Berserk v01.cbz");
        // A second book keeps the folder from looking empty, like an unplugged drive.
        own_cbz(&manga, "Monster v01.cbz");
        scan_all(&temp.db, &covers);
        let id = row_id(&temp.db, &book).unwrap();
        let old_series = series_of(&temp.db, &book).unwrap();

        let moved = comics.path().join("Berserk v01.cbz");
        fs::create_dir_all(comics.path()).unwrap();
        fs::rename(&book, &moved).unwrap();
        let report = scan_all(&temp.db, &covers);

        assert_eq!(changes_of(&report), (0, 1, 0));
        assert_eq!(row_id(&temp.db, &moved), Some(id));
        assert_eq!(directory_of(&temp.db, &moved), Some(to));
        let new_series = series_of(&temp.db, &moved).unwrap();
        assert_ne!(new_series, old_series, "the series of its new directory");
        assert_eq!(links(&temp.db), [(library, new_series)]);
        assert_eq!(
            series_rows(&temp.db),
            [
                ("Monster".to_string(), "monster".to_string()),
                ("Berserk".to_string(), "berserk".to_string())
            ],
            "the series it left, now empty, is gone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn nothing_leaves_a_directory_that_is_empty_missing_or_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let [emptied, gone, closed] = [TempDir::new(), TempDir::new(), TempDir::new()];
        for dir in [&emptied, &gone, &closed] {
            add_directory(&temp.db, dir.path(), None);
            own_cbz(
                dir,
                &format!("{} v01.cbz", dir.path().display()).replace('/', "_"),
            );
        }
        scan_all(&temp.db, &covers);

        // An unplugged drive leaves its mount point empty.
        for entry in fs::read_dir(emptied.path()).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::rename(gone.path(), gone.path().with_extension("away")).unwrap();
        fs::set_permissions(closed.path(), fs::Permissions::from_mode(0o000)).unwrap();
        let report = scan_all(&temp.db, &covers);
        fs::set_permissions(closed.path(), fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(gone.path().with_extension("away"), gone.path()).unwrap();

        assert_eq!(report.removed, 0);
        assert_eq!(count(&temp.db, "books"), 3);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cant_be_read_keeps_its_books() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let locked = own_cbz(&dir, "Locked/Berserk v01.cbz");
        let deleted = own_cbz(&dir, "Monster v01.cbz");
        own_cbz(&dir, "Pluto v01.cbz");
        scan_all(&temp.db, &covers);

        fs::remove_file(&deleted).unwrap();
        let folder = locked.parent().unwrap();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o000)).unwrap();
        let report = scan_all(&temp.db, &covers);
        fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(report.removed, 1, "the deleted one only");
        assert!(row_id(&temp.db, &locked).is_some());
        assert_eq!(row_id(&temp.db, &deleted), None);
    }

    #[test]
    fn only_a_full_scan_removes_the_books_in_no_directory() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        own_cbz(&dir, "Berserk v01.cbz");
        sql(
            &temp.db,
            "INSERT INTO series (name, match_key) VALUES ('Report', 'report')",
        );
        sql(
            &temp.db,
            "INSERT INTO books (title, author, format, language, tags, file_path, series) \
             VALUES ('Report', 'Unknown Author', 'pdf', 'Unknown', '', \
             '../Downloads/Report.pdf', (SELECT id FROM series WHERE name = 'Report'))",
        );

        let one_directory = scan(
            &temp.db,
            &covers,
            &ScanScope::folder(dir.path()),
            &Heartbeat::default(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(one_directory.removed, 0);
        let full = scan_all(&temp.db, &covers);

        assert_eq!(full.removed, 1);
        assert_eq!(count(&temp.db, "books"), 1);
        assert_eq!(
            series_rows(&temp.db),
            [("Berserk".to_string(), "berserk".to_string())],
            "its series went with it"
        );
    }

    #[test]
    fn a_book_in_no_directory_whose_file_is_gone_is_found_again_where_it_moved() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let directory = add_directory(&temp.db, dir.path(), None);
        let book = own_cbz(&dir, "Berserk v01.cbz");
        let hash = blake3::hash(&fs::read(&book).unwrap()).to_hex().to_string();
        // As a scan of `BOOK_DIRS` stored it, at a path whose file is gone.
        sql(
            &temp.db,
            &format!(
                "INSERT INTO books (title, author, format, language, tags, file_path, file_hash) \
                 VALUES ('Berserk', 'Unknown Author', 'cbz', 'Unknown', '', \
                 '../Downloads/Berserk v01.cbz', '{hash}')"
            ),
        );
        let id = row_id(&temp.db, Path::new("../Downloads/Berserk v01.cbz")).unwrap();

        let report = scan_all(&temp.db, &covers);

        assert_eq!(changes_of(&report), (0, 1, 0));
        assert_eq!(row_id(&temp.db, &book), Some(id));
        assert_eq!(directory_of(&temp.db, &book), Some(directory));
    }

    #[test]
    fn a_copy_never_takes_the_book_of_an_unplugged_drive() {
        let local = TempDir::new();
        let drive = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, local.path(), None);
        add_directory(&temp.db, drive.path(), None);
        let original = own_cbz(&drive, "Berserk v01.cbz");
        scan_all(&temp.db, &covers);
        let id = row_id(&temp.db, &original).unwrap();

        let copy = local.path().join("Berserk v01.cbz");
        fs::create_dir_all(local.path()).unwrap();
        fs::copy(&original, &copy).unwrap();
        // Unplugged: the mount point stays, empty.
        fs::remove_file(&original).unwrap();
        let report = scan(
            &temp.db,
            &covers,
            &ScanScope::folder(local.path()),
            &Heartbeat::default(),
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(changes_of(&report), (1, 0, 0));
        assert_eq!(row_id(&temp.db, &original), Some(id), "still the drive's");
    }

    #[test]
    fn a_full_scan_deletes_the_covers_no_row_names_but_the_fallback() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        fs::create_dir_all(&covers.folder).unwrap();
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let page = png(2, 3);
        let book = dir.write_cbz("Berserk v01.cbz", &[("001.png", &page)]);
        let library = add_library(&temp.db, "Manga");
        for name in ["chosen.png", "library.png", "orphan.png", DEFAULT_IMAGE] {
            fs::write(covers.path_of(name), name).unwrap();
        }
        fs::create_dir_all(covers.path_of("folder")).unwrap();
        scan(
            &temp.db,
            &covers,
            &ScanScope::folder(dir.path()),
            &Heartbeat::default(),
            &mut |_| {},
        )
        .unwrap();
        sql(&temp.db, "UPDATE series SET cover_image = 'chosen.png'");
        sql(
            &temp.db,
            &format!("UPDATE library SET cover_image = 'library.png' WHERE id = {library}"),
        );
        assert!(
            covers.path_of("orphan.png").exists(),
            "a directory scan tidies nothing"
        );

        scan_all(&temp.db, &covers);

        let book_cover = stored_cover(&temp.db, &book).expect("a cover");
        for kept in [
            book_cover.as_str(),
            "chosen.png",
            "library.png",
            DEFAULT_IMAGE,
            "folder",
        ] {
            assert!(covers.path_of(kept).exists(), "{kept} stays");
        }
        assert!(!covers.path_of("orphan.png").exists());
    }

    /// The canonical path of `dir`, made if it isn't there: the form an added directory is
    /// stored in, and the one a watcher reports.
    fn canonical(dir: &TempDir) -> PathBuf {
        fs::create_dir_all(dir.path()).unwrap();
        fs::canonicalize(dir.path()).unwrap()
    }

    fn scan_folder(db: &Database, covers: &Covers, folder: &Path) -> ScanReport {
        scan(
            db,
            covers,
            &ScanScope::folder(folder),
            &Heartbeat::default(),
            &mut |_| {},
        )
        .unwrap()
    }

    #[test]
    fn a_folder_scan_reads_that_folder_alone_and_removes_only_what_it_lost_there() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let root = canonical(&dir);
        add_directory(&temp.db, &root, None);
        for name in [
            "Manga/Berserk v01.cbz",
            "Manga/Berserk v02.cbz",
            "Comics/Saga v01.cbz",
        ] {
            own_cbz(&dir, name);
        }
        scan_all(&temp.db, &covers);
        fs::remove_file(root.join("Manga/Berserk v02.cbz")).unwrap();
        fs::remove_file(root.join("Comics/Saga v01.cbz")).unwrap();
        own_cbz(&dir, "Manga/Berserk v03.cbz");

        let report = scan_folder(&temp.db, &covers, &root.join("Manga"));

        assert_eq!((report.added, report.unchanged, report.removed), (1, 1, 1));
        assert!(row_id(&temp.db, &root.join("Manga/Berserk v03.cbz")).is_some());
        assert!(
            row_id(&temp.db, &root.join("Comics/Saga v01.cbz")).is_some(),
            "outside the folder"
        );
        assert_eq!(scan_all(&temp.db, &covers).removed, 1);
    }

    #[test]
    fn a_folder_inside_a_directory_can_be_an_image_folder_book_itself() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let root = canonical(&dir);
        add_directory(&temp.db, &root, None);
        dir.write("Pluto/1.jpg", b"page one");
        dir.write("Pluto/2.jpg", b"page two");

        let report = scan_folder(&temp.db, &covers, &root.join("Pluto"));

        assert_eq!(report.added, 1);
        assert!(row_id(&temp.db, &root.join("Pluto")).is_some());
    }

    #[test]
    fn a_folder_gone_behind_a_symlink_in_another_case_or_in_no_directory_is_not_read() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let root = canonical(&dir);
        add_directory(&temp.db, &root, None);
        own_cbz(&dir, "Manga/Berserk v01.cbz");
        own_cbz(&outside, "Saga v01.cbz");
        own_cbz(&outside, "Sub/Saga v02.cbz");
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut folders = vec![root.join("Gone"), root.join("manga"), canonical(&outside)];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), root.join("Link")).unwrap();
            folders.extend([root.join("Link"), root.join("Link/Sub")]);
        }
        scan_all(&temp.db, &covers);

        for folder in folders {
            let report = scan_folder(&temp.db, &covers, &folder);
            assert_eq!(report, ScanReport::default(), "{folder:?}");
        }

        assert_eq!(count(&temp.db, "books"), 1);
        assert!(row_id(&temp.db, &root.join("Manga/Berserk v01.cbz")).is_some());
    }

    #[test]
    fn a_file_moved_to_another_folder_keeps_its_row_when_that_folder_is_scanned_first() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        let root = canonical(&dir);
        add_directory(&temp.db, &root, None);
        own_cbz(&dir, "Inbox/Berserk v01.cbz");
        fs::create_dir_all(root.join("Manga")).unwrap();
        scan_all(&temp.db, &covers);
        let (old, new) = (
            root.join("Inbox/Berserk v01.cbz"),
            root.join("Manga/Berserk v01.cbz"),
        );
        let id = row_id(&temp.db, &old).unwrap();
        fs::rename(&old, &new).unwrap();

        let report = scan_folder(&temp.db, &covers, &root.join("Manga"));

        assert_eq!(changes_of(&report), (0, 1, 0));
        assert_eq!(row_id(&temp.db, &new), Some(id));
        let later = scan_folder(&temp.db, &covers, &root.join("Inbox"));
        assert_eq!(changes_of(&later), (0, 0, 0), "nothing left to remove");
    }

    #[test]
    fn a_file_is_hashed_in_chunks_with_a_sign_of_life_after_each() {
        let dir = TempDir::new();
        let bytes: Vec<u8> = (0..HASH_CHUNK * 5 / 2).map(|n| (n % 251) as u8).collect();
        let path = dir.write("Big v01.pdf", &bytes);
        let heartbeat = Heartbeat::default();

        let contents = read_contents(
            &Found {
                path,
                format: BookFormat::Pdf,
            },
            &heartbeat,
        )
        .unwrap();

        assert_eq!(contents.hash, blake3::hash(&bytes).to_hex().to_string());
        assert!(heartbeat.beats() >= 3, "{} beats", heartbeat.beats());
    }

    #[test]
    fn the_walk_beats_for_every_entry_and_tells_where_it_is() {
        let dir = TempDir::new();
        dir.write("Manga/Berserk v01.cbz", b"cbz");
        dir.write("Manga/Pluto/1.jpg", b"page");
        let heartbeat = Heartbeat::default();
        heartbeat.in_directory(1, dir.path());

        walk(dir.path(), dir.path(), &heartbeat);

        // Entering the directory, then Manga, Berserk v01.cbz, Pluto and 1.jpg.
        assert_eq!(heartbeat.beats(), 5);
        assert_eq!(
            heartbeat.spot().unwrap().path,
            dir.path().join("Manga/Pluto/1.jpg")
        );
    }

    #[test]
    fn a_scan_given_up_on_from_the_start_adds_and_removes_nothing() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let lost = own_cbz(&dir, "Berserk v01.cbz");
        scan_all(&temp.db, &covers);
        fs::remove_file(&lost).unwrap();
        own_cbz(&dir, "Berserk v02.cbz");
        fs::create_dir_all(&covers.folder).unwrap();
        fs::write(covers.path_of("orphan.png"), b"orphan").unwrap();
        let heartbeat = Heartbeat::default();
        heartbeat.give_up();

        let result = scan(&temp.db, &covers, &ScanScope::All, &heartbeat, &mut |_| {});

        assert!(matches!(result, Err(AppErrors::Scan(ScanError::GivenUp))));
        assert!(row_id(&temp.db, &lost).is_some(), "the lost row stays");
        assert_eq!(count(&temp.db, "books"), 1, "nothing added");
        assert!(covers.path_of("orphan.png").exists());
    }

    #[test]
    fn a_scan_given_up_on_after_its_first_book_keeps_that_one_and_removes_nothing() {
        let dir = TempDir::new();
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        add_directory(&temp.db, dir.path(), None);
        let lost = own_cbz(&dir, "Lost v01.cbz");
        scan_all(&temp.db, &covers);
        fs::remove_file(&lost).unwrap();
        // Pictures of their own, so each would get a cover file if it were read.
        for (name, width) in [("A v01.cbz", 2), ("B v01.cbz", 3), ("C v01.cbz", 4)] {
            dir.write_cbz(name, &[("001.png", &png(width, 3))]);
        }
        let heartbeat = Heartbeat::default();

        let result = scan(
            &temp.db,
            &covers,
            &ScanScope::All,
            &heartbeat,
            &mut |progress| {
                if progress.added == 1 {
                    heartbeat.give_up();
                }
            },
        );

        assert!(result.is_err());
        assert_eq!(
            count(&temp.db, "books"),
            2,
            "the lost row and the first book"
        );
        assert!(row_id(&temp.db, &lost).is_some());
        assert!(row_id(&temp.db, &dir.path().join("A v01.cbz")).is_some());
        assert_eq!(
            fs::read_dir(&covers.folder).unwrap().count(),
            1,
            "the books after it weren't even read"
        );
    }

    #[test]
    fn the_cover_folder_is_left_as_it_is_by_a_scan_given_up_on() {
        let cover_folder = TempDir::new();
        let covers = covers_in(&cover_folder);
        let temp = TempDatabase::open();
        fs::create_dir_all(&covers.folder).unwrap();
        fs::write(covers.path_of("orphan.png"), b"orphan").unwrap();
        let heartbeat = Heartbeat::default();
        heartbeat.give_up();

        assert!(remove_orphan_covers(&temp.db, &covers, &heartbeat).is_err());
        assert!(covers.path_of("orphan.png").exists());
    }

    #[test]
    fn a_write_of_a_scan_given_up_on_is_never_made() {
        let temp = TempDatabase::open();
        let heartbeat = Heartbeat::default();
        let insert = |name: &'static str| {
            move |conn: &Connection| {
                conn.execute(
                    "INSERT INTO series (name, match_key) VALUES (?1, lower(?1))",
                    [name],
                )
                .unwrap();
                Ok(())
            }
        };

        transaction(&temp.db, &heartbeat, insert("Berserk")).unwrap();
        heartbeat.give_up();
        let refused = transaction(&temp.db, &heartbeat, insert("Pluto"));

        assert!(matches!(refused, Err(AppErrors::Scan(ScanError::GivenUp))));
        assert_eq!(count(&temp.db, "series"), 1);
    }
}
