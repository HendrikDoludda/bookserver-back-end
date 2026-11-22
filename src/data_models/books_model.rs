use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    db::checked_unix_time,
    models::{
        BookMetadata, BookSeriesMetadata, Directory, LibraryMetadata, LibraryType, ReadingMode,
    },
};

/// One library as the library list sends it, and how many series it holds — never its stored
/// cover path: covers are reached through `GET /covers/library/{id}`.
#[derive(Debug, Serialize)]
pub struct LibrarySummary {
    pub name: String,
    pub library_type: LibraryType,
    pub description: Option<String>,
    pub series_count: i64,
}

impl LibrarySummary {
    pub fn new(library: LibraryMetadata, series_count: i64) -> Self {
        LibrarySummary {
            name: library.name,
            library_type: library.library_type,
            description: library.description,
            series_count,
        }
    }
}

/// One series as a library's series list sends it, and how many books it holds — never its
/// stored cover path: covers are reached through `GET /covers/series/{id}`.
#[derive(Debug, Serialize)]
pub struct SeriesSummary {
    pub name: String,
    pub description: Option<String>,
    pub start_release_year: Option<i64>,
    pub end_release_year: Option<i64>,
    pub book_count: i64,
}

impl SeriesSummary {
    pub fn new(series: BookSeriesMetadata, book_count: i64) -> Self {
        SeriesSummary {
            name: series.name,
            description: series.description,
            start_release_year: series.start_release_year,
            end_release_year: series.end_release_year,
            book_count,
        }
    }
}

/// One series as `GET /api/v1/series` lists it: its summary, and the libraries it is in.
#[derive(Debug, Serialize)]
pub struct SeriesListing {
    #[serde(flatten)]
    pub series: SeriesSummary,
    pub library_ids: Vec<i64>,
}

/// `POST /api/v1/libraries`.
#[derive(Debug, Deserialize)]
pub struct NewLibrary {
    pub name: String,
    pub library_type: LibraryType,
    pub description: Option<String>,
}

/// `PATCH /api/v1/libraries/{id}`: a field left out stays as it is, and `"description": null`
/// clears the description.
#[derive(Debug, Deserialize)]
pub struct LibraryChanges {
    pub name: Option<String>,
    pub library_type: Option<LibraryType>,
    #[serde(default, deserialize_with = "present")]
    pub description: Option<Option<String>>,
}

/// `PATCH /api/v1/series/{id}`: a field left out stays as it is, and `null` clears the
/// description or a release year.
#[derive(Debug, Deserialize)]
pub struct SeriesChanges {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub start_release_year: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    pub end_release_year: Option<Option<i64>>,
}

/// `POST /api/v1/directories`: the folder's path on the server, and whether a library of its own
/// receives its series — named `library_name`, else after the folder, and typed `library_type`,
/// which is required then.
#[derive(Debug, Deserialize)]
pub struct NewDirectory {
    pub path: String,
    pub create_library: bool,
    pub library_type: Option<LibraryType>,
    pub library_name: Option<String>,
}

/// One scanned folder as an admin gets it: its canonical path, and the library its series join.
#[derive(Debug, Serialize)]
pub struct DirectorySummary {
    pub path: String,
    pub library_id: Option<i64>,
}

impl From<Directory> for DirectorySummary {
    fn from(directory: Directory) -> Self {
        DirectorySummary {
            path: directory.path,
            library_id: directory.library_id,
        }
    }
}

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
#[derive(Debug, PartialEq, Serialize)]
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

/// `PUT /api/v1/series/{id}/cover`: the book whose cover the series shows.
#[derive(Debug, Deserialize)]
pub struct SeriesCoverChoice {
    pub book_id: i64,
}

/// `PUT /api/v1/libraries/{id}/cover`: the series whose cover the library shows.
#[derive(Debug, Deserialize)]
pub struct LibraryCoverChoice {
    pub series_id: i64,
}

/// Reads a PATCH field that can be cleared, together with `#[serde(default)]`: left out is
/// `None`, `null` is `Some(None)`, a value is `Some(Some(value))`.
// A plain `Option` reads a left-out field and `null` alike, so nothing could be cleared.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// One book as the app gets it — never the server's file path, the file's hash or the cover's
/// path: files and covers are reached by the book's id. Times are unix seconds.
#[derive(Debug, Serialize)]
pub struct BookSummary {
    pub title: String,
    pub author: String,
    /// `pdf`, `epub`, `cbz`, `image_comic` or `none`, as `BookFormat::as_str` writes it.
    pub format: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    /// The language's code, such as `de`; `None` while it is unknown.
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub series: Option<i64>,
    pub volume_number: Option<i64>,
    pub chapter_number: Option<i64>,
    pub page_number: Option<i64>,
    pub last_modified: Option<i64>,
    pub file_size: Option<u64>,
}

/// One book as `GET /books/{id}` answers it: its summary, and the mode the calling user opens it
/// in.
#[derive(Debug, Serialize)]
pub struct BookDetails {
    #[serde(flatten)]
    pub book: BookSummary,
    pub reading_mode: ReadingMode,
}

/// `PUT /api/v1/series/{id}/reading_mode`'s body. The mode is text, checked in code: as an enum,
/// an unknown one would answer `invalid_request` instead of its own key.
#[derive(Debug, Deserialize)]
pub struct ReadingModeChange {
    pub mode: String,
}

/// `PUT /api/v1/series/{id}/reading_mode`'s answer: the mode as stored.
#[derive(Debug, Serialize)]
pub struct ReadingModeSetting {
    pub mode: ReadingMode,
}

/// One entry of a book's page list. `width` and `height` are the image's pixels as stored (EXIF
/// rotation isn't applied), `None` when its header can't be read.
#[derive(Debug, Serialize)]
pub struct PageSummary {
    /// Counted from 1, as `GET /books/{id}/pages/{number}` takes it.
    pub number: usize,
    pub width: Option<usize>,
    pub height: Option<usize>,
}

impl From<BookMetadata> for BookSummary {
    fn from(book: BookMetadata) -> Self {
        let language = if book.language.is_unknown() {
            None
        } else {
            Some(book.language.to_code())
        };
        BookSummary {
            title: book.title,
            author: book.author,
            format: book.format.as_str().to_string(),
            tags: book.tags,
            description: book.description,
            language,
            page_count: book.page_count,
            series: book.series,
            volume_number: book.volume_number,
            chapter_number: book.chapter_number,
            page_number: book.page_number,
            last_modified: book.last_modified.and_then(checked_unix_time),
            file_size: book.file_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;
    use crate::models::{BookFormat, BookLanguage};

    /// A book the way the scanner writes one when a file tells it nothing more.
    fn scanned_book() -> BookMetadata {
        BookMetadata {
            title: "The Long Road".to_string(),
            author: "Unknown Author".to_string(),
            cover_image: Some("./cover_images/abc.jpg".to_string()),
            format: BookFormat::ImageComic,
            tags: vec![],
            description: None,
            file_path: "/Users/someone/books/The Long Road".to_string(),
            language: BookLanguage::Other("Unknown".to_string()),
            page_count: None,
            series: Some(4),
            volume_number: Some(1),
            chapter_number: None,
            page_number: None,
            file_hash: Some("abc".to_string()),
            last_modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            file_size: Some(2048),
            reading_direction: None,
            directory_id: Some(2),
        }
    }

    #[test]
    fn a_scanned_book_has_no_language_and_its_format_as_the_database_stores_it() {
        let summary = BookSummary::from(scanned_book());

        assert_eq!(summary.format, "image_comic");
        assert_eq!(summary.language, None);
        assert_eq!(summary.last_modified, Some(1_700_000_000));
        assert_eq!(summary.file_size, Some(2048));
        assert_eq!(summary.series, Some(4));
        assert_eq!(summary.volume_number, Some(1));
    }

    #[test]
    fn the_placeholder_language_is_unknown_in_any_case_and_a_real_one_goes_out_as_its_code() {
        let with_language = |language: BookLanguage| {
            BookSummary::from(BookMetadata {
                language,
                ..scanned_book()
            })
            .language
        };

        // As the scanner writes it, and as `from_code` reads it back from the database.
        for placeholder in ["Unknown", "unknown"] {
            assert_eq!(
                with_language(BookLanguage::Other(placeholder.to_string())),
                None
            );
        }
        assert_eq!(with_language(BookLanguage::German).as_deref(), Some("de"));
        assert_eq!(
            with_language(BookLanguage::Other("ko".to_string())).as_deref(),
            Some("ko")
        );
    }

    #[test]
    fn a_file_time_before_1970_goes_out_as_unknown() {
        let summary = BookSummary::from(BookMetadata {
            last_modified: Some(UNIX_EPOCH - Duration::from_secs(60)),
            ..scanned_book()
        });
        assert_eq!(summary.last_modified, None);
    }

    fn library_changes(body: serde_json::Value) -> LibraryChanges {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn a_library_change_tells_a_left_out_description_from_a_cleared_one() {
        let left_out = library_changes(serde_json::json!({ "name": "Manga" }));
        assert_eq!(left_out.description, None);
        assert_eq!(left_out.name.as_deref(), Some("Manga"));

        let cleared = library_changes(serde_json::json!({ "description": null }));
        assert_eq!(cleared.description, Some(None));

        let written = library_changes(serde_json::json!({ "description": "Japanese comics" }));
        assert_eq!(
            written.description,
            Some(Some("Japanese comics".to_string()))
        );
    }

    #[test]
    fn a_library_change_reads_the_type_by_its_stored_name_only() {
        let changes = library_changes(serde_json::json!({ "library_type": "comics" }));
        assert_eq!(changes.library_type, Some(LibraryType::Comics));

        let unknown = serde_json::from_value::<LibraryChanges>(
            serde_json::json!({ "library_type": "novels" }),
        );
        assert!(unknown.is_err());
    }
}
