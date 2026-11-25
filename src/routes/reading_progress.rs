//! A user's progress in a book: reading it, and storing a change to it.

use std::sync::Arc;

use crate::{
    data_models::books_model::ProgressChange,
    database_related_scripts::db_reading_progress,
    db::Database,
    error_types::AppErrors,
    models::{BookMetadata, Place, ReadingProgress},
};

/// The user's progress in the book — no place and unfinished while they have none; 404
/// `not_found` for a book that doesn't exist.
pub async fn progress_of(
    db: Arc<Database>,
    user_id: i64,
    book_id: i64,
) -> Result<ReadingProgress, AppErrors> {
    db.read(move |conn| {
        Database::get_entry_with_connection::<BookMetadata>(conn, book_id)?;
        Ok(db_reading_progress::find(conn, user_id, book_id)?.unwrap_or_default())
    })
    .await
}

/// Stores the change to the user's progress in the book, a place stamped with `now`, and answers
/// the progress as stored: 400 `invalid_request` for a change that changes nothing or a place that
/// isn't whole and inside its total, 404 `not_found` for a book that doesn't exist.
pub async fn save_progress(
    db: Arc<Database>,
    user_id: i64,
    book_id: i64,
    change: ProgressChange,
    now: i64,
) -> Result<ReadingProgress, AppErrors> {
    let place = checked_place(change.position, change.total)?;
    let finished = change.finished;
    if place.is_none() && finished.is_none() {
        return Err(AppErrors::InvalidRequest);
    }
    db.transact(move |conn| {
        // The foreign key would refuse the row as well, but as a 500.
        Database::get_entry_with_connection::<BookMetadata>(conn, book_id)?;
        db_reading_progress::save(conn, user_id, book_id, place, finished, now)?;
        Ok(db_reading_progress::find(conn, user_id, book_id)?.unwrap_or_default())
    })
    .await
}

/// The place a change sends: none when it leaves both fields out, one when it sends both with the
/// position inside `0..total`; anything else is refused.
fn checked_place(
    position: Option<Option<i64>>,
    total: Option<Option<i64>>,
) -> Result<Option<Place>, AppErrors> {
    match (position, total) {
        (None, None) => Ok(None),
        (Some(Some(position)), Some(Some(total))) if (0..total).contains(&position) => {
            Ok(Some(Place { position, total }))
        }
        _ => Err(AppErrors::InvalidRequest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error_types::DatabaseError, test_support::TempDatabase};

    fn change(position: Option<Option<i64>>, total: Option<Option<i64>>) -> ProgressChange {
        ProgressChange {
            position,
            total,
            finished: None,
        }
    }

    #[test]
    fn a_place_is_both_fields_with_the_position_inside_the_total() {
        assert_eq!(checked_place(None, None).unwrap(), None);
        assert_eq!(
            checked_place(Some(Some(0)), Some(Some(1))).unwrap(),
            Some(Place {
                position: 0,
                total: 1
            })
        );
        assert_eq!(
            checked_place(Some(Some(199)), Some(Some(200))).unwrap(),
            Some(Place {
                position: 199,
                total: 200
            })
        );
        for (position, total) in [
            (Some(Some(3)), None),
            (None, Some(Some(10))),
            (Some(None), Some(None)),
            (Some(None), Some(Some(10))),
            (Some(Some(10)), Some(Some(10))),
            (Some(Some(-1)), Some(Some(10))),
            (Some(Some(0)), Some(Some(0))),
        ] {
            assert!(
                matches!(
                    checked_place(position, total),
                    Err(AppErrors::InvalidRequest)
                ),
                "{position:?} of {total:?}"
            );
        }
    }

    /// A book of its own series, as the scanner stores one; answers its id.
    fn add_book(temp: &TempDatabase) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO series (name) VALUES ('Berserk')", [])
                    .unwrap();
                let series = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, series) \
                     VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', '/b/v01.cbz', ?1)",
                    [series],
                )
                .unwrap();
                let book = conn.last_insert_rowid();
                crate::test_support::insert_user(conn, "alice");
                Ok(book)
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_saved_change_is_answered_and_read_back() {
        let temp = TempDatabase::open();
        let book = add_book(&temp);
        let alice = 1;

        let unread = progress_of(temp.db.clone(), alice, book).await.unwrap();
        assert_eq!(unread, ReadingProgress::default());

        let saved = save_progress(
            temp.db.clone(),
            alice,
            book,
            change(Some(Some(4)), Some(Some(200))),
            1_700_000_000,
        )
        .await
        .unwrap();
        let expected = ReadingProgress {
            position: Some(4),
            total: Some(200),
            finished: false,
        };
        assert_eq!(saved, expected);
        assert_eq!(
            progress_of(temp.db.clone(), alice, book).await.unwrap(),
            expected
        );

        let marked = save_progress(
            temp.db.clone(),
            alice,
            book,
            ProgressChange {
                finished: Some(true),
                ..change(None, None)
            },
            1_700_000_100,
        )
        .await
        .unwrap();
        assert_eq!(
            marked,
            ReadingProgress {
                finished: true,
                ..expected
            }
        );
    }

    #[tokio::test]
    async fn a_change_of_nothing_a_broken_place_or_an_unknown_book_is_refused() {
        let temp = TempDatabase::open();
        let book = add_book(&temp);
        let alice = 1;
        let save = |book_id, change| save_progress(temp.db.clone(), alice, book_id, change, 1);

        for refused in [
            change(None, None),
            change(Some(Some(3)), None),
            change(Some(Some(10)), Some(Some(10))),
            change(Some(None), Some(None)),
        ] {
            let result = save(book, refused).await;
            assert!(matches!(result, Err(AppErrors::InvalidRequest)));
        }
        let unknown = save(book + 1, change(Some(Some(0)), Some(Some(1)))).await;
        assert!(matches!(
            unknown,
            Err(AppErrors::Database(DatabaseError::EntryNotFound))
        ));
        assert!(matches!(
            progress_of(temp.db.clone(), alice, book + 1).await,
            Err(AppErrors::Database(DatabaseError::EntryNotFound))
        ));

        assert_eq!(
            progress_of(temp.db.clone(), alice, book).await.unwrap(),
            ReadingProgress::default(),
            "a refused change stores nothing"
        );
    }
}
