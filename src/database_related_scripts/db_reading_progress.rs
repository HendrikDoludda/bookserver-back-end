//! The `reading_progress` table: each user's place in each book. Its key is two columns (user and
//! book), so it gets these dedicated functions instead of the generic `Extract` / `Update` traits.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::{
    database_related_scripts::db::{counts_by_id, logged},
    error_types::DatabaseError,
    models::{Place, ReadingProgress},
};

/// The user's progress in the book; `None` while they have none.
pub fn find(
    conn: &Connection,
    user_id: i64,
    book_id: i64,
) -> Result<Option<ReadingProgress>, DatabaseError> {
    conn.query_row(
        "SELECT position, total, finished FROM reading_progress \
         WHERE user_id = ?1 AND book_id = ?2",
        params![user_id, book_id],
        progress_of_row,
    )
    .optional()
    .map_err(logged(
        "reading",
        "reading_progress",
        DatabaseError::QueryFailure,
    ))
}

/// Stores a change to the user's progress in the book: their `place`, and whether they
/// `finished` it. What is `None` stays as stored — on a new row: no place, unfinished. A place
/// also stamps `last_read_at` with `now`.
pub fn save(
    conn: &Connection,
    user_id: i64,
    book_id: i64,
    place: Option<Place>,
    finished: Option<bool>,
    now: i64,
) -> Result<(), DatabaseError> {
    let position = place.map(|place| place.position);
    let total = place.map(|place| place.total);
    let last_read_at = place.map(|_| now);
    conn.execute(
        "INSERT INTO reading_progress (user_id, book_id, position, total, finished, last_read_at) \
         VALUES (?1, ?2, ?3, ?4, COALESCE(?5, 0), ?6) \
         ON CONFLICT (user_id, book_id) DO UPDATE SET \
         position = COALESCE(excluded.position, position), \
         total = COALESCE(excluded.total, total), \
         finished = COALESCE(?5, finished), \
         last_read_at = COALESCE(excluded.last_read_at, last_read_at)",
        params![user_id, book_id, position, total, finished, last_read_at],
    )
    .map_err(logged(
        "inserting into",
        "reading_progress",
        DatabaseError::InsertionFailure,
    ))?;
    Ok(())
}

/// The user's progress in each book of the series that they have any in, by book id.
pub fn of_series(
    conn: &Connection,
    user_id: i64,
    series_id: i64,
) -> Result<HashMap<i64, ReadingProgress>, DatabaseError> {
    let mut stmt = conn
        .prepare(
            "SELECT reading_progress.book_id, reading_progress.position, reading_progress.total, \
             reading_progress.finished FROM reading_progress \
             JOIN books ON books.id = reading_progress.book_id \
             WHERE reading_progress.user_id = ?1 AND books.series = ?2",
        )
        .map_err(logged(
            "reading",
            "reading_progress",
            DatabaseError::TaskPreparationFailure,
        ))?;
    let rows = stmt
        .query_map(params![user_id, series_id], |row| {
            Ok((row.get::<_, i64>("book_id")?, progress_of_row(row)?))
        })
        .map_err(logged(
            "reading",
            "reading_progress",
            DatabaseError::QueryFailure,
        ))?;
    rows.collect::<Result<HashMap<_, _>, _>>().map_err(logged(
        "reading",
        "reading_progress",
        DatabaseError::NextRowFailure,
    ))
}

/// How many books of each series in the library the user finished, by series id; a series they
/// finished none of is absent.
pub fn finished_per_series_in_library(
    conn: &Connection,
    user_id: i64,
    library_id: i64,
) -> Result<HashMap<i64, i64>, DatabaseError> {
    counts_by_id(
        conn,
        "SELECT books.series, COUNT(*) FROM reading_progress \
         JOIN books ON books.id = reading_progress.book_id \
         JOIN library_elements ON library_elements.series_id = books.series \
         WHERE reading_progress.user_id = ?1 AND reading_progress.finished = 1 \
         AND library_elements.library_id = ?2 \
         GROUP BY books.series",
        params![user_id, library_id],
        "reading_progress",
    )
}

fn progress_of_row(row: &Row) -> rusqlite::Result<ReadingProgress> {
    Ok(ReadingProgress {
        position: row.get("position")?,
        total: row.get("total")?,
        finished: row.get("finished")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{insert_user, memory_connection};

    fn insert_series(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_book(conn: &Connection, series_id: i64, file_path: &str) -> i64 {
        conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path, series) \
             VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', ?1, ?2)",
            params![file_path, series_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn place(position: i64, total: i64) -> Option<Place> {
        Some(Place { position, total })
    }

    fn last_read_at(conn: &Connection, user_id: i64, book_id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT last_read_at FROM reading_progress WHERE user_id = ?1 AND book_id = ?2",
            [user_id, book_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn a_first_place_starts_unfinished_and_stamps_when_it_was_read() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let berserk = insert_series(&conn, "Berserk");
        let book = insert_book(&conn, berserk, "/books/Berserk v01.cbz");
        assert_eq!(find(&conn, alice, book).unwrap(), None);

        save(&conn, alice, book, place(4, 200), None, 1_700_000_000).unwrap();

        assert_eq!(
            find(&conn, alice, book).unwrap(),
            Some(ReadingProgress {
                position: Some(4),
                total: Some(200),
                finished: false,
            })
        );
        assert_eq!(last_read_at(&conn, alice, book), Some(1_700_000_000));
    }

    #[test]
    fn a_change_keeps_what_it_leaves_out() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let berserk = insert_series(&conn, "Berserk");
        let book = insert_book(&conn, berserk, "/books/Berserk v01.cbz");
        save(&conn, alice, book, place(4, 200), None, 1_700_000_000).unwrap();

        save(&conn, alice, book, None, Some(true), 1_700_000_100).unwrap();
        assert_eq!(
            find(&conn, alice, book).unwrap(),
            Some(ReadingProgress {
                position: Some(4),
                total: Some(200),
                finished: true,
            }),
            "marking it keeps the place"
        );
        assert_eq!(
            last_read_at(&conn, alice, book),
            Some(1_700_000_000),
            "marking by hand is no reading"
        );

        save(&conn, alice, book, place(5, 200), None, 1_700_000_200).unwrap();
        assert_eq!(
            find(&conn, alice, book).unwrap(),
            Some(ReadingProgress {
                position: Some(5),
                total: Some(200),
                finished: true,
            }),
            "a place keeps the mark"
        );
        assert_eq!(last_read_at(&conn, alice, book), Some(1_700_000_200));

        save(
            &conn,
            alice,
            book,
            place(0, 200),
            Some(false),
            1_700_000_300,
        )
        .unwrap();
        assert_eq!(
            find(&conn, alice, book).unwrap(),
            Some(ReadingProgress {
                position: Some(0),
                total: Some(200),
                finished: false,
            }),
            "both at once"
        );
    }

    #[test]
    fn a_book_marked_read_before_it_was_opened_has_no_place() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let berserk = insert_series(&conn, "Berserk");
        let book = insert_book(&conn, berserk, "/books/Berserk v01.cbz");

        save(&conn, alice, book, None, Some(true), 1_700_000_000).unwrap();

        assert_eq!(
            find(&conn, alice, book).unwrap(),
            Some(ReadingProgress {
                position: None,
                total: None,
                finished: true,
            })
        );
        assert_eq!(last_read_at(&conn, alice, book), None);
    }

    #[test]
    fn a_series_progress_holds_only_this_users_books_of_that_series() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        let berserk = insert_series(&conn, "Berserk");
        let monster = insert_series(&conn, "Monster");
        let first = insert_book(&conn, berserk, "/books/Berserk v01.cbz");
        let second = insert_book(&conn, berserk, "/books/Berserk v02.cbz");
        let elsewhere = insert_book(&conn, monster, "/books/Monster v01.cbz");
        save(&conn, alice, first, place(9, 10), Some(true), 1).unwrap();
        save(&conn, alice, elsewhere, place(3, 10), None, 1).unwrap();
        save(&conn, bob, second, place(2, 10), None, 1).unwrap();

        let progress = of_series(&conn, alice, berserk).unwrap();

        assert_eq!(
            progress,
            HashMap::from([(
                first,
                ReadingProgress {
                    position: Some(9),
                    total: Some(10),
                    finished: true,
                }
            )])
        );
    }

    #[test]
    fn finished_books_are_counted_per_series_for_this_user_and_library_only() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        conn.execute(
            "INSERT INTO library (library_name, library_type) VALUES ('Manga', 'comics'), \
             ('Other', 'comics')",
            [],
        )
        .unwrap();
        let (manga, other) = (1, 2);
        let berserk = insert_series(&conn, "Berserk");
        let monster = insert_series(&conn, "Monster");
        for (library, series) in [(manga, berserk), (other, monster)] {
            conn.execute(
                "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                [library, series],
            )
            .unwrap();
        }
        let books: Vec<i64> = (1..=3)
            .map(|volume| insert_book(&conn, berserk, &format!("/books/Berserk v{volume}.cbz")))
            .collect();
        let elsewhere = insert_book(&conn, monster, "/books/Monster v01.cbz");
        save(&conn, alice, books[0], None, Some(true), 1).unwrap();
        save(&conn, alice, books[1], None, Some(true), 1).unwrap();
        save(&conn, alice, books[2], place(1, 10), None, 1).unwrap();
        save(&conn, alice, elsewhere, None, Some(true), 1).unwrap();
        save(&conn, bob, books[2], None, Some(true), 1).unwrap();

        assert_eq!(
            finished_per_series_in_library(&conn, alice, manga).unwrap(),
            HashMap::from([(berserk, 2)]),
            "neither the unfinished book, Monster in the other library, nor bob's"
        );
        assert_eq!(
            finished_per_series_in_library(&conn, bob, manga).unwrap(),
            HashMap::from([(berserk, 1)])
        );
    }
}
