//! Which mode a user reads a book in, and storing their choice for its series.

use std::sync::Arc;

use rusqlite::Connection;

use crate::{
    database_related_scripts::db_reading_modes,
    db::Database,
    error_types::{AppErrors, DatabaseError, ReadingModeError},
    models::{BookFormat, BookMetadata, BookSeriesMetadata, ReadingMode},
};

/// The mode `user_id` opens `book` in: their choice for its series, else the direction its file
/// states, else its reader's own way — scrolling for a PDF, left to right for the rest.
pub fn mode_for(
    conn: &Connection,
    user_id: i64,
    book: &BookMetadata,
) -> Result<ReadingMode, DatabaseError> {
    let chosen = match book.series {
        Some(series_id) => db_reading_modes::find(conn, user_id, series_id)?,
        None => None,
    };
    Ok(chosen
        .or(book.reading_direction.map(ReadingMode::from))
        .unwrap_or(match book.format {
            BookFormat::Pdf => ReadingMode::Continuous,
            BookFormat::Epub | BookFormat::Cbz | BookFormat::ImageComic | BookFormat::None => {
                ReadingMode::Ltr
            }
        }))
}

/// Stores the mode `name` names as the user's choice for the series, and answers it: 400
/// `invalid_reading_mode` for a mode that doesn't exist, 404 `not_found` for a series that
/// doesn't.
pub async fn set_mode(
    db: Arc<Database>,
    user_id: i64,
    series_id: i64,
    name: &str,
) -> Result<ReadingMode, AppErrors> {
    let mode = ReadingMode::from_name(name).ok_or(ReadingModeError::InvalidReadingMode)?;
    db.transact(move |conn| {
        // The foreign key would refuse the row as well, but as a 500.
        Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
        db_reading_modes::save(conn, user_id, series_id, mode)?;
        Ok(mode)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{BookLanguage, ReadingDirection},
        test_support::{insert_user, memory_connection, TempDatabase},
    };

    fn insert_series(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    /// A book of `format` in `series`, its file stating `direction`.
    fn book(
        format: BookFormat,
        series: Option<i64>,
        direction: Option<ReadingDirection>,
    ) -> BookMetadata {
        BookMetadata {
            title: "Berserk".to_string(),
            author: "Kentaro Miura".to_string(),
            cover_image: None,
            format,
            tags: vec![],
            description: None,
            file_path: "/books/Berserk v01".to_string(),
            language: BookLanguage::Japanese,
            page_count: None,
            series,
            volume_number: Some(1),
            chapter_number: None,
            page_number: None,
            file_hash: None,
            last_modified: None,
            file_size: None,
            reading_direction: direction,
        }
    }

    #[test]
    fn without_a_choice_or_a_direction_a_pdf_scrolls_and_the_rest_go_left_to_right() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let series = insert_series(&conn, "berserk");
        for (format, expected) in [
            (BookFormat::Pdf, ReadingMode::Continuous),
            (BookFormat::Cbz, ReadingMode::Ltr),
            (BookFormat::ImageComic, ReadingMode::Ltr),
            (BookFormat::Epub, ReadingMode::Ltr),
            (BookFormat::None, ReadingMode::Ltr),
        ] {
            let mode = mode_for(&conn, alice, &book(format.clone(), Some(series), None));
            assert_eq!(mode.unwrap(), expected, "{format:?}");
        }
    }

    #[test]
    fn the_direction_the_file_states_beats_the_format() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let series = insert_series(&conn, "berserk");

        for (direction, expected) in [
            (ReadingDirection::Rtl, ReadingMode::Rtl),
            (ReadingDirection::Ltr, ReadingMode::Ltr),
        ] {
            let book = book(BookFormat::Cbz, Some(series), Some(direction));
            assert_eq!(mode_for(&conn, alice, &book).unwrap(), expected);
        }
    }

    #[test]
    fn the_users_choice_for_the_series_beats_the_file_and_holds_only_for_them() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        let series = insert_series(&conn, "berserk");
        let manga = book(BookFormat::Cbz, Some(series), Some(ReadingDirection::Rtl));
        let scan = book(BookFormat::Pdf, Some(series), None);

        db_reading_modes::save(&conn, alice, series, ReadingMode::Continuous).unwrap();

        assert_eq!(
            mode_for(&conn, alice, &manga).unwrap(),
            ReadingMode::Continuous
        );
        assert_eq!(
            mode_for(&conn, alice, &scan).unwrap(),
            ReadingMode::Continuous,
            "every book of the series"
        );
        assert_eq!(mode_for(&conn, bob, &manga).unwrap(), ReadingMode::Rtl);
        assert_eq!(
            mode_for(&conn, bob, &scan).unwrap(),
            ReadingMode::Continuous
        );
    }

    #[test]
    fn a_book_without_a_series_follows_its_file_and_format() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");

        let lone = book(BookFormat::Cbz, None, Some(ReadingDirection::Rtl));
        assert_eq!(mode_for(&conn, alice, &lone).unwrap(), ReadingMode::Rtl);
        let lone = book(BookFormat::Pdf, None, None);
        assert_eq!(
            mode_for(&conn, alice, &lone).unwrap(),
            ReadingMode::Continuous
        );
    }

    fn add(temp: &TempDatabase) -> (i64, i64) {
        temp.db
            .setup_new_transaction(|conn| {
                Ok((insert_user(conn, "alice"), insert_series(conn, "berserk")))
            })
            .unwrap()
    }

    fn stored(temp: &TempDatabase, user_id: i64, series_id: i64) -> Option<ReadingMode> {
        temp.db
            .setup_new_transaction(|conn| Ok(db_reading_modes::find(conn, user_id, series_id)?))
            .unwrap()
    }

    #[tokio::test]
    async fn a_mode_is_stored_and_answered_and_a_later_one_replaces_it() {
        let temp = TempDatabase::open();
        let (alice, series) = add(&temp);

        let answer = set_mode(Arc::clone(&temp.db), alice, series, "rtl").await;
        assert_eq!(answer.unwrap(), ReadingMode::Rtl);
        assert_eq!(stored(&temp, alice, series), Some(ReadingMode::Rtl));

        set_mode(Arc::clone(&temp.db), alice, series, "continuous")
            .await
            .unwrap();
        assert_eq!(stored(&temp, alice, series), Some(ReadingMode::Continuous));
    }

    #[tokio::test]
    async fn a_mode_that_does_not_exist_is_refused_and_stores_nothing() {
        let temp = TempDatabase::open();
        let (alice, series) = add(&temp);

        for name in ["sideways", "RTL", " rtl", ""] {
            let answer = set_mode(Arc::clone(&temp.db), alice, series, name).await;
            assert!(
                matches!(
                    answer,
                    Err(AppErrors::ReadingMode(ReadingModeError::InvalidReadingMode))
                ),
                "{name:?}: {answer:?}"
            );
        }
        assert_eq!(stored(&temp, alice, series), None);
    }

    #[tokio::test]
    async fn a_series_that_does_not_exist_is_not_found() {
        let temp = TempDatabase::open();
        let (alice, series) = add(&temp);

        let answer = set_mode(Arc::clone(&temp.db), alice, series + 1, "ltr").await;

        assert!(
            matches!(
                answer,
                Err(AppErrors::Database(DatabaseError::EntryNotFound))
            ),
            "{answer:?}"
        );
    }
}
