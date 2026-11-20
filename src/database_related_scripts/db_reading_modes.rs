//! The `reading_modes` table, and how a `ReadingMode` and a book's `ReadingDirection` are stored.
//! Its key is two columns (user and series), so it gets these dedicated functions instead of the
//! generic `Extract` / `Update` traits.

use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    Connection, OptionalExtension, ToSql,
};

use crate::{
    database_related_scripts::db::logged,
    error_types::DatabaseError,
    models::{ReadingDirection, ReadingMode},
};

impl ToSql for ReadingMode {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for ReadingMode {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        ReadingMode::from_name(value.as_str()?).ok_or(FromSqlError::InvalidType)
    }
}

impl ToSql for ReadingDirection {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for ReadingDirection {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "rtl" => Ok(ReadingDirection::Rtl),
            "ltr" => Ok(ReadingDirection::Ltr),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

/// The mode the user chose for the series; `None` while they have chosen none.
pub fn find(
    conn: &Connection,
    user_id: i64,
    series_id: i64,
) -> Result<Option<ReadingMode>, DatabaseError> {
    conn.query_row(
        "SELECT mode FROM reading_modes WHERE user_id = ?1 AND series_id = ?2",
        params![user_id, series_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(logged(
        "reading",
        "reading_modes",
        DatabaseError::QueryFailure,
    ))
}

/// Stores the user's mode for the series, replacing an earlier one.
pub fn save(
    conn: &Connection,
    user_id: i64,
    series_id: i64,
    mode: ReadingMode,
) -> Result<(), DatabaseError> {
    conn.execute(
        "INSERT INTO reading_modes (user_id, series_id, mode) VALUES (?1, ?2, ?3) \
         ON CONFLICT (user_id, series_id) DO UPDATE SET mode = excluded.mode",
        params![user_id, series_id, mode],
    )
    .map_err(logged(
        "inserting into",
        "reading_modes",
        DatabaseError::InsertionFailure,
    ))?;
    Ok(())
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

    fn rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM reading_modes", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_mode_is_found_once_saved_and_a_later_save_replaces_it() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let series = insert_series(&conn, "berserk");
        assert_eq!(find(&conn, alice, series).unwrap(), None);

        save(&conn, alice, series, ReadingMode::Rtl).unwrap();
        assert_eq!(find(&conn, alice, series).unwrap(), Some(ReadingMode::Rtl));

        save(&conn, alice, series, ReadingMode::Continuous).unwrap();
        assert_eq!(
            find(&conn, alice, series).unwrap(),
            Some(ReadingMode::Continuous)
        );
        assert_eq!(rows(&conn), 1, "a later save updates the one row");
    }

    #[test]
    fn a_mode_belongs_to_one_user_and_one_series() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        let berserk = insert_series(&conn, "berserk");
        let monster = insert_series(&conn, "monster");

        save(&conn, alice, berserk, ReadingMode::Rtl).unwrap();
        save(&conn, bob, monster, ReadingMode::Ltr).unwrap();

        assert_eq!(find(&conn, alice, berserk).unwrap(), Some(ReadingMode::Rtl));
        assert_eq!(find(&conn, alice, monster).unwrap(), None);
        assert_eq!(find(&conn, bob, berserk).unwrap(), None);
        assert_eq!(find(&conn, bob, monster).unwrap(), Some(ReadingMode::Ltr));
    }

    #[test]
    fn a_mode_for_a_series_that_does_not_exist_is_refused() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");

        assert!(matches!(
            save(&conn, alice, 999, ReadingMode::Ltr),
            Err(DatabaseError::InsertionFailure)
        ));
        assert_eq!(rows(&conn), 0);
    }

    #[test]
    fn modes_and_directions_are_stored_as_their_names_and_read_back() {
        let conn = memory_connection();
        for mode in [ReadingMode::Rtl, ReadingMode::Ltr, ReadingMode::Continuous] {
            let stored: String = conn
                .query_row("SELECT ?1", [mode], |row| row.get(0))
                .unwrap();
            assert_eq!(stored, mode.as_str());
            let read: ReadingMode = conn
                .query_row("SELECT ?1", [mode.as_str()], |row| row.get(0))
                .unwrap();
            assert_eq!(read, mode);
        }
        for direction in [ReadingDirection::Rtl, ReadingDirection::Ltr] {
            let stored: String = conn
                .query_row("SELECT ?1", [direction], |row| row.get(0))
                .unwrap();
            assert_eq!(stored, direction.as_str());
            let read: ReadingDirection = conn
                .query_row("SELECT ?1", [direction.as_str()], |row| row.get(0))
                .unwrap();
            assert_eq!(read, direction);
        }
    }

    #[test]
    fn other_text_is_an_error_not_a_mode_or_a_direction() {
        let conn = memory_connection();
        for text in ["sideways", "RTL", ""] {
            let mode = conn.query_row("SELECT ?1", [text], |row| row.get::<_, ReadingMode>(0));
            assert!(mode.is_err(), "{text:?} read as {mode:?}");
        }
        let direction = conn.query_row("SELECT 'continuous'", [], |row| {
            row.get::<_, ReadingDirection>(0)
        });
        assert!(direction.is_err(), "got {direction:?}");
    }
}
