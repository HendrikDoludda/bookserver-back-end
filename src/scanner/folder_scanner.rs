//! The scan: each added directory walked once, each book file and image folder in it stored in a
//! transaction of its own, and what couldn't be read or stored reported.

use std::{
    ffi::OsStr,
    fs::{self, File},
    io,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use blake3::Hasher;
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;
use tracing::warn;
use walkdir::WalkDir;

use crate::book_pages;
use crate::data_models::models::{
    BookDatabaseColumns, BookFormat, BookLanguage, BookMetadata, BookSeriesMetadata, Directory,
    FileExtractedMetadata, ParsedName, QuerySeparator, SeriesLibraryConnection, WithId,
};
use crate::data_models::scan_model::{
    ScanFailure, ScanProblem, ScanProgress, ScanReport, ScanScope,
};
use crate::db::Database;
use crate::error_types::{AppErrors, DatabaseError};
use crate::scanner::comic_info;
use crate::scanner::cover_image_retriever::{cover_for, Covers};

// Filename patterns, compiled once on first use and then reused.
static VOLUME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:v|vol|volume)[ _]?(\d+)\b").unwrap());

static CHAPTER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:c|chap|chapter)[ _]?(\d+)\b").unwrap());

static PAGE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:p|pa|page)[ _]?(\d+)\b").unwrap());

/// Scans the directories `scope` reads and answers what it did, once its last book is written.
/// `progress` hears how many books the walk found, then how far the scan got after each book.
pub fn scan(
    db: &Database,
    covers: &Covers,
    scope: &ScanScope,
    progress: &mut dyn FnMut(ScanProgress),
) -> Result<ScanReport, AppErrors> {
    let directories: Vec<WithId<Directory>> = db
        .get_all_entries::<Directory>()?
        .into_iter()
        .filter(|directory| scope.reads(directory.id))
        .collect();
    // Every directory is walked before any book is read, so the progress knows how many there are.
    let walks: Vec<Walk> = directories
        .iter()
        .map(|directory| walk(Path::new(&directory.data.path)))
        .collect();
    let found = walks.iter().map(|walk| walk.books.len()).sum();
    progress(ScanProgress {
        found,
        ..ScanProgress::default()
    });
    let mut report = ScanReport::default();
    for (directory, walk) in directories.iter().zip(walks) {
        record(
            &mut report,
            directory.id,
            Path::new(&directory.data.path),
            walk,
            |book| store(db, covers, directory.id, book),
            |report| {
                progress(ScanProgress {
                    found,
                    added: report.added,
                    unchanged: report.unchanged,
                    failed: report.failures.len(),
                })
            },
        );
    }
    Ok(report)
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
}

/// What a walk found: the books, and the folders it couldn't read.
#[derive(Debug, Default)]
struct Walk {
    books: Vec<Found>,
    problems: Vec<(PathBuf, ScanProblem)>,
}

/// The book files and image folders under `root`, in name order, and what couldn't be read on the
/// way. A symlink is never followed and a hidden entry never read; images lying in `root` itself
/// make no book.
fn walk(root: &Path) -> Walk {
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
    let entries = WalkDir::new(root)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !is_hidden(entry.file_name()));
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
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
fn is_hidden(name: &OsStr) -> bool {
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
    Added,
    Unchanged,
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
    let failure = |path: &Path, problem| ScanFailure {
        directory_id,
        relative_path: path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned(),
        error_key: problem,
    };
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
            Ok(Stored::Unchanged) => report.unchanged += 1,
            Err(problem) => report.failures.push(failure(&found.path, problem)),
        }
        after_each(report);
    }
}

/// Stores `found` as a book of the directory, unless it is stored already. Its file is read first
/// — hash, cover, reading direction — and one transaction then finds or makes its series and
/// inserts it, so a failure leaves neither behind.
fn store(
    db: &Database,
    covers: &Covers,
    directory_id: i64,
    found: &Found,
) -> Result<Stored, ScanProblem> {
    let file_path = found.path.to_string_lossy().into_owned();
    let stored = db
        .search_for_single_row::<BookMetadata>(
            &[BookDatabaseColumns::FilePath],
            &[&file_path],
            QuerySeparator::And,
        )
        .map_err(|_| ScanProblem::StoreFailed)?;
    if stored.is_some() {
        return Ok(Stored::Unchanged);
    }

    let unreadable = |path: &Path, error: &dyn std::fmt::Display| {
        warn!("{path:?} can't be read: {error}");
        ScanProblem::FileUnreadable
    };
    // An image folder's own file is its page 1.
    let source = match found.format {
        BookFormat::ImageComic => book_pages::folder_page(&found.path, 1)
            .map_err(|error| unreadable(&found.path, &error))?,
        _ => found.path.clone(),
    };
    let file = File::open(&source).map_err(|error| unreadable(&source, &error))?;
    let metadata = get_file_metadata(&file, &source);
    let file_hash = hash_file(file).map_err(|error| unreadable(&source, &error))?;
    let cover_image = cover_for(covers, &found.path, &found.format, Some(&file_hash));
    let reading_direction = comic_info::reading_direction(&found.format, &found.path);
    let parsed = parse_name(&found.name());

    let mut book = BookMetadata {
        title: parsed.title,
        author: metadata.author,
        format: found.format.clone(),
        tags: vec![],
        description: None,
        file_path,
        language: metadata.language_code,
        page_count: metadata.page_number,
        cover_image,
        series: None,
        volume_number: parsed.volume_number,
        chapter_number: parsed.chapter_number,
        page_number: parsed.page_number,
        file_hash: Some(file_hash),
        last_modified: metadata.modified_date,
        file_size: metadata.file_size,
        reading_direction,
        directory_id: Some(directory_id),
    };
    db.setup_new_transaction(move |conn| {
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

/// The series a book titled `title` joins: the one whose match key is the title lowercased and that
/// holds a book of the same directory, else a new one named as found and put into the directory's
/// library, if it has one. A failing lookup fails the book, as "none" would make a second series.
fn series_for(conn: &Connection, title: &str, directory_id: i64) -> Result<i64, DatabaseError> {
    // Lowercased in Rust, which knows every alphabet: "ÄRGER v02" finds "Ärger".
    let key = title.to_lowercase();
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

fn get_file_metadata(file: &File, path: &Path) -> FileExtractedMetadata {
    let metadata = file
        .metadata()
        .inspect_err(|error| warn!("{path:?}'s size and time can't be read: {error}"))
        .ok();
    let modified_date = metadata.as_ref().and_then(|metadata| {
        metadata
            .modified()
            .inspect_err(|error| warn!("{path:?}'s time can't be read: {error}"))
            .ok()
    });
    FileExtractedMetadata {
        file_size: metadata.map(|metadata| metadata.len()),
        modified_date,
        page_number: None,
        language_code: BookLanguage::Other("Unknown".to_string()),
        author: "Unknown Author".to_string(),
    }
}

/// Splits a name like `My_Series v02 c13` into a clean title plus the volume / chapter / page
/// numbers found in it.
fn parse_name(name: &str) -> ParsedName {
    let volume = extract_number(&VOLUME_REGEX, name);
    let chapter = extract_number(&CHAPTER_REGEX, name);
    let page = extract_number(&PAGE_REGEX, name);

    let cleaned = PAGE_REGEX
        .replace_all(
            &CHAPTER_REGEX.replace_all(&VOLUME_REGEX.replace_all(name, ""), ""),
            "",
        )
        .to_string();

    let title = cleaned
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    ParsedName {
        title,
        volume_number: volume,
        chapter_number: chapter,
        page_number: page,
    }
}

fn extract_number(regex: &Regex, text: &str) -> Option<i64> {
    regex
        .captures(text)
        .and_then(|cap| cap.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok())
}

fn hash_file(mut file: File) -> io::Result<String> {
    let mut hasher = Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::SystemTime;

    use super::*;
    use crate::data_models::models::{LibraryMetadata, LibraryType};
    use crate::test_support::{TempDatabase, TempDir};

    const RIGHT_TO_LEFT: &[u8] = b"<ComicInfo><Manga>YesAndRightToLeft</Manga></ComicInfo>";

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

    /// The books `walk` finds under `root`: each one's path relative to it, and its format.
    fn books_under(root: &Path) -> Vec<(String, String)> {
        walk(root)
            .books
            .iter()
            .map(|found| {
                (
                    found
                        .path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
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

        let walks = [&missing, &closed, &open].map(|root| walk(root));
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
            walk(dir.path()),
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
                unchanged: 0,
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
            &ScanScope::Directories(BTreeSet::from([present, missing])),
            &mut |progress| heard.push(progress),
        )
        .unwrap();

        assert_eq!(
            first,
            ScanReport {
                added: 2,
                unchanged: 0,
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
                    unchanged: 0,
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

        let second = scan(&temp.db, &covers, &ScanScope::All, &mut |_| {}).unwrap();

        assert_eq!(
            second,
            ScanReport {
                added: 1,
                unchanged: 2,
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
}
