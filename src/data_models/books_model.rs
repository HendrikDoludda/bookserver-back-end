use serde::{Deserialize, Serialize};

use crate::{
    db::checked_unix_time,
    models::{BookMetadata, BookSeriesMetadata, LibraryMetadata, ReadingMode},
};

/// One library as the library list sends it: its row, and how many series it holds.
#[derive(Debug, Serialize)]
pub struct LibrarySummary {
    #[serde(flatten)]
    pub library: LibraryMetadata,
    pub series_count: i64,
}

/// One series as a library's series list sends it: its row, and how many books it holds.
#[derive(Debug, Serialize)]
pub struct SeriesSummary {
    #[serde(flatten)]
    pub series: BookSeriesMetadata,
    pub book_count: i64,
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
}
