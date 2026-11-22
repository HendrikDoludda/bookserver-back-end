use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use blake3::Hasher;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::{fs, sync::Semaphore};
use tracing::{error, warn};

use crate::data_models::models::{
    BookDatabaseColumns, BookFormat, BookLanguage, BookMetadata, BookSeriesMetadata,
    ColumnSelector, DatabaseTypes, Directory, FileExtractedMetadata, ParsedName,
    SeriesLibraryConnection,
};
use crate::db::Database;
use crate::error_types::FolderScannerError;
use crate::scanner::comic_info;
use crate::scanner::cover_image_retriever::{cover_for, Covers};

// Filename patterns, compiled once on first use and then reused.
static VOLUME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:v|vol|volume)[ _]?(\d+)\b").unwrap());

static CHAPTER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:c|chap|chapter)[ _]?(\d+)\b").unwrap());

static PAGE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:p|pa|page)[ _]?(\d+)\b").unwrap());

/// Scans every added directory for supported files and creates database entries for them.
pub async fn scan_all_folders(db: Arc<Database>, covers: Covers) -> anyhow::Result<()> {
    let directories = Arc::clone(&db)
        .read(|conn| {
            Ok(Database::get_all_entries_with_connection::<Directory>(
                conn,
            )?)
        })
        .await?;

    for directory in directories {
        scan_folder(
            PathBuf::from(directory.data.path),
            db.clone(),
            directory.id,
            covers.clone(),
        )
        .await?;
    }

    Ok(())
}

/// Walks the tree of the directory `directory_id`, at `path`, and creates a book entry for each
/// supported file. File processing runs on the blocking thread pool, gated by a semaphore.
pub async fn scan_folder(
    path: PathBuf,
    db: Arc<Database>,
    directory_id: i64,
    covers: Covers,
) -> Result<(), FolderScannerError> {
    let semaphore = Arc::new(Semaphore::new(1)); // limit concurrency
    let mut directories = vec![path];

    while let Some(dir) = directories.pop() {
        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|_| FolderScannerError::MissingDirectory)?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|_| FolderScannerError::FileNotFound)?
        {
            let entry_path = entry.path();
            // The entry's own type, so a symlink is never followed: what it leads to may lie
            // outside the directory, and a book read there would show its page 1 as a cover.
            let Ok(kind) = entry.file_type().await else {
                continue;
            };

            if kind.is_dir() {
                directories.push(entry_path);
            } else if kind.is_file() {
                let db = db.clone();
                let covers = covers.clone();
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| FolderScannerError::PermitCreationFailed);

                tokio::spawn(async move {
                    let _permit = permit;

                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        create_book_entry(&entry_path, &db, directory_id, &covers)
                    })
                    .await
                    {
                        error!("Task join error: {e}");
                    }
                });
            }
        }
    }

    Ok(())
}

/// Creates the database entry (plus series and cover image) for one file of the directory
/// `directory_id`. Unsupported files and files already in the database are skipped.
fn create_book_entry(
    path: &Path,
    db: &Database,
    directory_id: i64,
    covers: &Covers,
) -> Result<(), FolderScannerError> {
    if !is_valid_file_type(path) {
        return Ok(());
    }

    let format = get_book_format(path)?;

    let file = File::open(path).map_err(|_| FolderScannerError::FileNotFound)?;
    let metadata = get_file_metadata(&file, path);

    let file_hash = match hash_file(file).map_err(|_| FolderScannerError::FileHashingFailed) {
        Ok(hash) => Some(hash),
        Err(e) => {
            warn!("Failed to hash the file {:?}", e);
            None
        }
    };

    let final_path = get_final_folder_path(path, format.as_str());
    if check_folder_path_exists_in_db(&final_path, db) {
        // TODO: also detect modified files and update their entry.
        return Ok(());
    }

    let parsed_name = get_final_file_name(&final_path);
    let series_id = get_series_id(&parsed_name.title, db, directory_id)?;
    let cover_image = cover_for(covers, &final_path, &format, file_hash.as_deref());
    let reading_direction = comic_info::reading_direction(&format, &final_path);

    let metadata_entry = BookMetadata {
        title: parsed_name.title,
        author: metadata.author,
        format,
        tags: vec![],
        description: None,
        file_path: final_path.to_string_lossy().to_string(),
        language: metadata.language_code,
        page_count: metadata.page_number,
        cover_image,
        series: Some(series_id),
        volume_number: parsed_name.volume_number,
        chapter_number: parsed_name.chapter_number,
        page_number: parsed_name.page_number,
        file_hash,
        last_modified: metadata.modified_date,
        file_size: metadata.file_size,
        reading_direction,
        directory_id: Some(directory_id),
    };

    db.insert(&metadata_entry)
        .map_err(|_| FolderScannerError::BookInsertionFailed)?;

    Ok(())
}

fn get_file_metadata(file: &File, path: &Path) -> FileExtractedMetadata {
    let metadata = match file
        .metadata()
        .map_err(|_| FolderScannerError::MetadataExtractionFailed)
    {
        Ok(m) => Some(m),
        Err(e) => {
            warn!("Could not extract {:?}'s metadata  {:?}", path, e);
            None
        }
    };

    if let Some(metadata) = metadata {
        let modified_time = match metadata
            .modified()
            .map_err(|_| FolderScannerError::UnsupportedOperatingSystem)
        {
            Ok(date) => Some(date),
            Err(e) => {
                warn!("Failed to retrieve the modified date from file: {:?}", e);
                None
            }
        };
        let file_size = metadata.len();

        FileExtractedMetadata {
            file_size: Some(file_size),
            modified_date: modified_time,
            page_number: None,
            language_code: BookLanguage::Other("Unknown".to_string()),
            author: "Unknown Author".to_string(),
        }
    } else {
        FileExtractedMetadata {
            file_size: None,
            modified_date: None,
            page_number: None,
            language_code: BookLanguage::Other("Unknown".to_string()),
            author: "Unknown Author".to_string(),
        }
    }
}

fn get_final_folder_path(path: &Path, format: &str) -> PathBuf {
    if (BookFormat::ImageComic.as_str() == format) && path.parent().is_some() {
        let parent = path.parent().unwrap();
        if parent.is_dir() {
            return parent.to_path_buf();
        }
    }
    path.to_path_buf()
}

fn check_folder_path_exists_in_db(path: &Path, db: &Database) -> bool {
    let result = db.get_id_from_table(
        DatabaseTypes::Books,
        ColumnSelector::Book(BookDatabaseColumns::FilePath),
        &path.to_string_lossy().to_string(),
    );
    matches!(result, Ok(Some(_)))
}

/// Splits a filename like `My_Series v02 c13` into a clean title plus the
/// volume / chapter / page numbers found in it.
fn get_final_file_name(path: &Path) -> ParsedName {
    let file_name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let volume = extract_number(&VOLUME_REGEX, &file_name);
    let chapter = extract_number(&CHAPTER_REGEX, &file_name);
    let page = extract_number(&PAGE_REGEX, &file_name);

    let cleaned = PAGE_REGEX
        .replace_all(
            &CHAPTER_REGEX.replace_all(&VOLUME_REGEX.replace_all(&file_name, ""), ""),
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

fn is_valid_file_type(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("pdf" | "epub" | "cbz" | "jpg" | "jpeg" | "png")
    )
}

fn get_book_format(path: &Path) -> Result<BookFormat, FolderScannerError> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("pdf") => Ok(BookFormat::Pdf),
        Some("epub") => Ok(BookFormat::Epub),
        Some("cbz") => Ok(BookFormat::Cbz),
        Some("jpg" | "jpeg" | "png") => Ok(BookFormat::ImageComic),
        _ => Err(FolderScannerError::FileTypeNotRecognized),
    }
}

fn hash_file(mut file: File) -> io::Result<String> {
    let mut hasher = Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// The series a book titled `title` joins: the one whose match key is the title lowercased and that
/// holds a book of the same directory, else a new one named as found and put into the directory's
/// library, if it has one. A database error fails the book, as a lookup that answered "none" would
/// make a second series.
fn get_series_id(title: &str, db: &Database, directory_id: i64) -> Result<i64, FolderScannerError> {
    // Lowercased in Rust, which knows every alphabet: "ÄRGER v02" finds "Ärger".
    let key = title.to_lowercase();
    db.setup_new_transaction(|conn| {
        if let Some(id) =
            Database::series_of_directory_by_match_key_with_connection(conn, directory_id, &key)?
        {
            return Ok(id);
        }
        // Read here rather than when the scan started: the library may have been deleted since,
        // and linking to it would fail the book.
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
    })
    .map_err(|error| {
        warn!("no series for {title:?}: {error}");
        FolderScannerError::SeriesInsertionFailed
    })
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::data_models::models::{LibraryMetadata, LibraryType};
    use crate::test_support::{TempDatabase, TempDir};

    const RIGHT_TO_LEFT: &[u8] = b"<ComicInfo><Manga>YesAndRightToLeft</Manga></ComicInfo>";

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

        create_book_entry(&berserk, &temp.db, directory, &covers).unwrap();
        create_book_entry(&pluto, &temp.db, directory, &covers).unwrap();

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

        create_book_entry(&pdf, &temp.db, directory, &covers).unwrap();

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

        create_book_entry(&epub, &temp.db, directory, &covers).unwrap();

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

    /// Scans each of `names` as a one-page CBZ in `dir`, a file of the directory `directory`;
    /// answers their paths.
    fn scan_books(dir: &TempDir, db: &Database, directory: i64, names: &[&str]) -> Vec<PathBuf> {
        let covers = covers_in(dir);
        names
            .iter()
            .map(|name| {
                let book = dir.write_cbz(name, &[("001.jpg", b"not a picture")]);
                create_book_entry(&book, db, directory, &covers).unwrap();
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

        let result = create_book_entry(&second, &temp.db, directory, &covers_in(&dir));

        assert!(matches!(
            result,
            Err(FolderScannerError::SeriesInsertionFailed)
        ));
        assert_eq!(count(&temp.db, "series"), 1);
        assert_eq!(
            count(&temp.db, "books"),
            1,
            "no row, so the next scan tries the file again"
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

        create_book_entry(&cbz, &temp.db, directory, &covers).unwrap();

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

        let result = create_book_entry(&book, &temp.db, directory, &covers_in(&dir));

        assert!(matches!(
            result,
            Err(FolderScannerError::SeriesInsertionFailed)
        ));
        assert_eq!(count(&temp.db, "series"), 0);
        assert_eq!(count(&temp.db, "books"), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_scan_follows_no_symlink_out_of_its_directory() {
        use std::os::unix::fs::symlink;

        let books = TempDir::new();
        let outside = TempDir::new();
        let cover_folder = TempDir::new();
        let temp = TempDatabase::open();
        let kept = books.write_cbz("Berserk v01.cbz", &[("001.jpg", b"not a picture")]);
        let private = outside.write_cbz("Private v01.cbz", &[("001.jpg", b"not a picture")]);
        // The scan returns while its last file may still be read. Each folder holding a book
        // also holds `zz/notes.txt`, read after the folder's own files, so that file is no book.
        books.write("zz/notes.txt", b"not a book");
        outside.write("zz/notes.txt", b"not a book");
        symlink(&private, books.path().join("Leak v01.cbz")).unwrap();
        symlink(outside.path(), books.path().join("Linked")).unwrap();
        let directory = add_directory(&temp.db, books.path(), None);

        scan_folder(
            books.path().to_path_buf(),
            Arc::clone(&temp.db),
            directory,
            covers_in(&cover_folder),
        )
        .await
        .unwrap();

        let stored: Vec<String> = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .prepare("SELECT file_path FROM books")
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap())
            })
            .unwrap();
        assert_eq!(stored, [kept.to_string_lossy()]);
    }
}
