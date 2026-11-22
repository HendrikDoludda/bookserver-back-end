use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use r2d2::{ManageConnection, Pool};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, Result, ToSql, TransactionBehavior};
use serde::Serialize;

use crate::data_models::models::{
    ColumnSelector, Comparison, DatabaseTypes, QuerySeparator, WithId,
};
use crate::database_related_scripts::{
    db_delete::Delete, db_search::Search, db_update::Update, extract::Extract, insert::Insert,
    migrations, translation_seed,
};
use crate::error_types::{AppErrors, DatabaseError};

/// Handle to the SQLite database: an r2d2 connection pool plus every query the
/// app runs. Most operations come in two flavours: a plain method that takes a
/// connection from the pool, and a `*_with_connection` variant that runs on a
/// caller-supplied connection so several operations can share one transaction
/// (see [`Database::setup_new_transaction`]).
// TODO: decide whether filtering happens here or in the frontend. Leaning towards
// the backend: pass filter options in with the request and apply them in the query.
pub struct Database {
    pool: r2d2::Pool<SqliteConnectionManager>,
    folder: PathBuf,
}

/// A series' books in reading order: by volume, then chapter — a book without the number after
/// those with one — then title, then id.
// SQLite sorts NULL before every number, so `IS NULL` comes first to put it last.
const BOOK_READING_ORDER: &str = "volume_number IS NULL, volume_number, \
    chapter_number IS NULL, chapter_number, title COLLATE NOCASE, id";

/// Settings every connection needs. SQLite applies them per connection, not per file,
/// so a connection that skips this runs without foreign keys.
pub fn configure(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        ",
    )
}

/// For `map_err` on a rusqlite call: logs the cause with what was being done, and answers
/// `error` in its place — the variant alone can't say why a query failed.
pub(crate) fn logged(
    action: &'static str,
    what: &'static str,
    error: DatabaseError,
) -> impl FnOnce(rusqlite::Error) -> DatabaseError {
    move |cause| {
        tracing::error!("{action} {what} failed: {cause}");
        error
    }
}

/// The conditions after `WHERE` for the generic `Search` and `Delete`: one `column <op> ?` per
/// condition, joined by `separator`. An empty list is refused rather than matching every row.
pub(crate) fn where_conditions<C: AsRef<str>>(
    conditions: &[(C, Comparison)],
    values: &[&dyn ToSql],
    separator: QuerySeparator,
) -> Result<String, DatabaseError> {
    if conditions.is_empty() || conditions.len() != values.len() {
        return Err(DatabaseError::InvalidParameters);
    }
    Ok(conditions
        .iter()
        .map(|(column, comparison)| format!("{} {} ?", column.as_ref(), comparison.as_str()))
        .collect::<Vec<String>>()
        .join(separator.as_str()))
}

/// Each column compared with `=`.
pub(crate) fn each_equal<C>(columns: &[C]) -> Vec<(&C, Comparison)> {
    columns
        .iter()
        .map(|column| (column, Comparison::Equal))
        .collect()
}

impl Database {
    pub fn new() -> Result<Self, DatabaseError> {
        // Ensure the database folder exists
        std::fs::create_dir_all("./data/databases/")
            .map_err(|_| DatabaseError::DirectoryCreationFailure)?;

        Self::open(Path::new("./data/databases/app_data.sqlite"))
    }

    /// Opens (or creates) the database at `path`, applies any pending migrations and syncs the
    /// shipped translations.
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        let manager = SqliteConnectionManager::file(path).with_init(|conn| configure(conn));

        {
            let mut conn = manager.connect().map_err(logged(
                "opening",
                "the database",
                DatabaseError::FailedToConnectToDatabase,
            ))?;

            migrations::run(&mut conn)?;
            translation_seed::run(&mut conn)?;
        }

        let pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .map_err(|_| DatabaseError::ConnectionPoolFailure)?;

        let _conn = pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;

        // A directory being added is compared with this folder by real location, so its symlinks
        // are resolved here, once the file exists.
        let mut folder = std::fs::canonicalize(path).map_err(|cause| {
            tracing::error!("locating the database's folder failed: {cause}");
            DatabaseError::DatabaseInitializationFailure
        })?;
        folder.pop();

        Ok(Self { pool, folder })
    }

    /// The folder the database's file lies in, with every symlink resolved.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn insert(&self, data: &impl Insert) -> Result<i64, DatabaseError> {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::insert_with_connection(&conn, data)
    }

    pub fn insert_with_connection(
        conn: &Connection,
        data: &impl Insert,
    ) -> Result<i64, DatabaseError> {
        let id: i64 = data.insert(conn)?;
        Ok(id)
    }

    /// Returns the number of rows changed: 0 when no row has that key.
    pub fn update_value<U>(
        &self,
        column: U::Column,
        new_value: &dyn ToSql,
        id: i64,
    ) -> Result<usize, DatabaseError>
    where
        U: Update,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::update_value_with_connection::<U>(&conn, column, new_value, id)
    }

    pub fn update_value_with_connection<U>(
        conn: &Connection,
        column: U::Column,
        new_value: &dyn ToSql,
        id: i64,
    ) -> Result<usize, DatabaseError>
    where
        U: Update,
    {
        U::update(conn, column, new_value, id)
    }

    pub fn update_multiple_values<U>(
        &self,
        columns: &[U::Column],
        new_values: &[&dyn ToSql],
        id: i64,
    ) -> Result<usize, DatabaseError>
    where
        U: Update,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::update_multiple_values_with_connection::<U>(&conn, columns, new_values, id)
    }

    pub fn update_multiple_values_with_connection<U>(
        conn: &Connection,
        columns: &[U::Column],
        new_values: &[&dyn ToSql],
        id: i64,
    ) -> Result<usize, DatabaseError>
    where
        U: Update,
    {
        U::update_multiple(conn, columns, new_values, id)
    }

    /// Deletes the row whose key column equals `id`; returns the number of rows deleted.
    pub fn remove_entry(&self, db_type: DatabaseTypes, id: i64) -> Result<usize, DatabaseError> {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::remove_entry_with_connection(&conn, db_type, id)
    }

    pub fn remove_entry_with_connection(
        conn: &Connection,
        db_type: DatabaseTypes,
        id: i64,
    ) -> Result<usize, DatabaseError> {
        let key = db_type
            .get_key_column()
            .ok_or(DatabaseError::NoSingleKeyColumn)?;
        let query = format!(
            "DELETE FROM {} WHERE {} = ?1",
            db_type.get_table_name(),
            key
        );
        conn.execute(&query, rusqlite::params![id]).map_err(logged(
            "deleting from",
            db_type.get_table_name(),
            DatabaseError::ConnectionExecutableFailure,
        ))
    }

    pub fn remove_entry_with_connection_based_on_columns<T>(
        conn: &Connection,
        columns: &[T::Column],
        values: &[&dyn ToSql],
        query_separator: QuerySeparator,
    ) -> Result<usize, DatabaseError>
    where
        T: Delete,
    {
        T::delete(conn, columns, values, query_separator)
    }

    /// Deletes the rows matching `conditions`, each column compared with its value by its
    /// `Comparison`; returns the number of rows deleted.
    pub fn remove_entries_where_with_connection<T>(
        conn: &Connection,
        conditions: &[(T::Column, Comparison)],
        values: &[&dyn ToSql],
        query_separator: QuerySeparator,
    ) -> Result<usize, DatabaseError>
    where
        T: Delete,
    {
        T::delete_where(conn, conditions, values, query_separator)
    }

    pub fn get_entry<T>(&self, id: i64) -> Result<T, DatabaseError>
    where
        T: Extract,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::get_entry_with_connection::<T>(&conn, id)
    }

    pub fn get_entry_with_connection<T>(conn: &Connection, id: i64) -> Result<T, DatabaseError>
    where
        T: Extract,
    {
        T::extract(conn, id)?.ok_or(DatabaseError::EntryNotFound)
    }

    /// Fetch many rows by id in a single query, each paired with its id.
    /// Returns an empty Vec for an empty id list (no query is run).
    pub fn get_entries_batch<T>(&self, ids: &[i64]) -> Result<Vec<WithId<T>>, DatabaseError>
    where
        T: Extract + Serialize,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::get_entries_batch_with_connection::<T>(&conn, ids)
    }

    pub fn get_entries_batch_with_connection<T>(
        conn: &Connection,
        ids: &[i64],
    ) -> Result<Vec<WithId<T>>, DatabaseError>
    where
        T: Extract + Serialize,
    {
        T::extract_batch(conn, ids)
    }

    /// Every row of the table in key order, each paired with its key.
    pub fn get_all_entries<T>(&self) -> Result<Vec<WithId<T>>, DatabaseError>
    where
        T: Extract + Serialize,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::get_all_entries_with_connection::<T>(&conn)
    }

    pub fn get_all_entries_with_connection<T>(
        conn: &Connection,
    ) -> Result<Vec<WithId<T>>, DatabaseError>
    where
        T: Extract + Serialize,
    {
        T::extract_all(conn)
    }

    pub fn search_for_single_row<T>(
        &self,
        columns: &[T::Column],
        values: &[&dyn ToSql],
        query_separator: QuerySeparator,
    ) -> Result<Option<T>, DatabaseError>
    where
        T: Search,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::search_for_single_row_with_connection::<T>(&conn, columns, values, query_separator)
    }

    pub fn search_for_single_row_with_connection<T>(
        conn: &Connection,
        columns: &[T::Column],
        values: &[&dyn ToSql],
        query_separator: QuerySeparator,
    ) -> Result<Option<T>, DatabaseError>
    where
        T: Search,
    {
        T::search_for_row(conn, columns, values, query_separator)
    }

    pub fn search_for_multiple_rows<T>(
        &self,
        columns: &[T::Column],
        values: &[&dyn ToSql],
        query_separator: QuerySeparator,
    ) -> Result<Vec<T>, DatabaseError>
    where
        T: Search,
    {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::search_for_multiple_rows_with_connection::<T>(&conn, columns, values, query_separator)
    }

    pub fn search_for_multiple_rows_with_connection<T>(
        conn: &Connection,
        columns: &[T::Column],
        values: &[&dyn ToSql],
        query_separator: QuerySeparator,
    ) -> Result<Vec<T>, DatabaseError>
    where
        T: Search,
    {
        T::search_for_multiple_rows(conn, columns, values, query_separator)
    }

    /// The ids of a series' books in reading order: by volume, then chapter — a book without
    /// the number after those with one — then title, then id.
    pub fn get_books_in_series(&self, series_id: i64) -> Result<Vec<i64>, DatabaseError> {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::get_books_in_series_with_connection(&conn, series_id)
    }

    pub fn get_books_in_series_with_connection(
        conn: &Connection,
        series_id: i64,
    ) -> Result<Vec<i64>, DatabaseError> {
        let query = format!("SELECT id FROM books WHERE series = ?1 ORDER BY {BOOK_READING_ORDER}");
        let mut stmt = conn.prepare(&query).map_err(logged(
            "reading",
            "books",
            DatabaseError::TaskPreparationFailure,
        ))?;
        let rows = stmt
            .query_map(rusqlite::params![series_id], |row| row.get::<_, i64>("id"))
            .map_err(logged("reading", "books", DatabaseError::QueryFailure))?;
        rows.collect::<Result<Vec<i64>, _>>().map_err(logged(
            "reading",
            "books",
            DatabaseError::NextRowFailure,
        ))
    }

    /// The stored cover of a series' first book, in reading order, that has one.
    pub fn first_book_cover_with_connection(
        conn: &Connection,
        series_id: i64,
    ) -> Result<Option<String>, DatabaseError> {
        let query = format!(
            "SELECT cover_image FROM books WHERE series = ?1 AND cover_image IS NOT NULL \
             ORDER BY {BOOK_READING_ORDER} LIMIT 1"
        );
        conn.query_row(&query, [series_id], |row| row.get(0))
            .optional()
            .map_err(logged("reading", "books", DatabaseError::QueryFailure))
    }

    pub fn remove_entry_from_library_elements(
        &self,
        series_id: i64,
        library_id: i64,
    ) -> Result<(), DatabaseError> {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::remove_entry_from_library_elements_with_connection(&conn, series_id, library_id)
    }

    pub fn remove_entry_from_library_elements_with_connection(
        conn: &Connection,
        series_id: i64,
        library_id: i64,
    ) -> Result<(), DatabaseError> {
        let query = "DELETE FROM library_elements WHERE series_id = ?1 AND library_id = ?2";
        conn.execute(query, rusqlite::params![series_id, library_id])
            .map_err(logged(
                "deleting from",
                "library_elements",
                DatabaseError::ConnectionExecutableFailure,
            ))?;
        Ok(())
    }

    pub fn get_series_entries_in_library(
        &self,
        library_id: i64,
    ) -> Result<Vec<i64>, DatabaseError> {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        Self::get_series_entries_in_library_with_connection(&conn, library_id)
    }

    /// The ids of a library's series, by name.
    pub fn get_series_entries_in_library_with_connection(
        conn: &Connection,
        library_id: i64,
    ) -> Result<Vec<i64>, DatabaseError> {
        let mut stmt = conn
            .prepare(
                "SELECT library_elements.series_id FROM library_elements \
                 JOIN series ON series.id = library_elements.series_id \
                 WHERE library_elements.library_id = ?1 \
                 ORDER BY series.name COLLATE NOCASE, series.id",
            )
            .map_err(logged(
                "reading",
                "library_elements",
                DatabaseError::OperationFailure,
            ))?;
        let rows = stmt
            .query_map(rusqlite::params![library_id], |row| {
                row.get::<_, i64>("series_id")
            })
            .map_err(logged(
                "reading",
                "library_elements",
                DatabaseError::OperationFailure,
            ))?;
        rows.collect::<Result<Vec<i64>, _>>().map_err(logged(
            "reading",
            "library_elements",
            DatabaseError::OperationFailure,
        ))
    }

    /// How many series each library holds, by library id; a library without series is absent.
    pub fn count_series_per_library_with_connection(
        conn: &Connection,
    ) -> Result<HashMap<i64, i64>, DatabaseError> {
        counts_by_id(
            conn,
            "SELECT library_id, COUNT(*) FROM library_elements GROUP BY library_id",
            rusqlite::params![],
            "library_elements",
        )
    }

    /// How many books each series of a library holds, by series id; a series without books is
    /// absent.
    pub fn count_books_per_series_in_library_with_connection(
        conn: &Connection,
        library_id: i64,
    ) -> Result<HashMap<i64, i64>, DatabaseError> {
        counts_by_id(
            conn,
            "SELECT books.series, COUNT(*) FROM books \
             JOIN library_elements ON library_elements.series_id = books.series \
             WHERE library_elements.library_id = ?1 \
             GROUP BY books.series",
            rusqlite::params![library_id],
            "books",
        )
    }

    /// How many books each series holds, by series id; a series without books is absent.
    pub fn count_books_per_series_with_connection(
        conn: &Connection,
    ) -> Result<HashMap<i64, i64>, DatabaseError> {
        counts_by_id(
            conn,
            "SELECT series, COUNT(*) FROM books WHERE series IS NOT NULL GROUP BY series",
            rusqlite::params![],
            "books",
        )
    }

    /// The ids of every series, by name.
    pub fn all_series_ids_by_name_with_connection(
        conn: &Connection,
    ) -> Result<Vec<i64>, DatabaseError> {
        let mut stmt = conn
            .prepare("SELECT id FROM series ORDER BY name COLLATE NOCASE, id")
            .map_err(logged(
                "reading",
                "series",
                DatabaseError::TaskPreparationFailure,
            ))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(logged("reading", "series", DatabaseError::QueryFailure))?;
        rows.collect::<Result<Vec<i64>, _>>().map_err(logged(
            "reading",
            "series",
            DatabaseError::NextRowFailure,
        ))
    }

    /// The libraries each series is in, by series id, each list by library id; a series in no
    /// library is absent.
    pub fn libraries_per_series_with_connection(
        conn: &Connection,
    ) -> Result<HashMap<i64, Vec<i64>>, DatabaseError> {
        let mut stmt = conn
            .prepare(
                "SELECT series_id, library_id FROM library_elements \
                 ORDER BY series_id, library_id",
            )
            .map_err(logged(
                "reading",
                "library_elements",
                DatabaseError::TaskPreparationFailure,
            ))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(logged(
                "reading",
                "library_elements",
                DatabaseError::QueryFailure,
            ))?;
        let mut libraries: HashMap<i64, Vec<i64>> = HashMap::new();
        for pair in rows {
            let (series_id, library_id) = pair.map_err(logged(
                "reading",
                "library_elements",
                DatabaseError::NextRowFailure,
            ))?;
            libraries.entry(series_id).or_default().push(library_id);
        }
        Ok(libraries)
    }

    /// The series whose match key is `key` and that holds a book of the directory — the oldest,
    /// should there be several — or `None`.
    pub fn series_of_directory_by_match_key_with_connection(
        conn: &Connection,
        directory_id: i64,
        key: &str,
    ) -> Result<Option<i64>, DatabaseError> {
        conn.query_row(
            "SELECT id FROM series WHERE match_key = ?1 AND EXISTS \
             (SELECT 1 FROM books WHERE books.series = series.id AND books.directory_id = ?2) \
             ORDER BY id LIMIT 1",
            rusqlite::params![key, directory_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(logged("reading", "series", DatabaseError::QueryFailure))
    }

    /// The id and stored path of every book in no directory, by id.
    pub fn books_without_directory_with_connection(
        conn: &Connection,
    ) -> Result<Vec<(i64, String)>, DatabaseError> {
        let mut stmt = conn
            .prepare("SELECT id, file_path FROM books WHERE directory_id IS NULL ORDER BY id")
            .map_err(logged(
                "reading",
                "books",
                DatabaseError::TaskPreparationFailure,
            ))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(logged("reading", "books", DatabaseError::QueryFailure))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(logged(
            "reading",
            "books",
            DatabaseError::NextRowFailure,
        ))
    }

    /// The series that hold a book of the directory, by id.
    pub fn series_of_directory_with_connection(
        conn: &Connection,
        directory_id: i64,
    ) -> Result<Vec<i64>, DatabaseError> {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT series FROM books \
                 WHERE directory_id = ?1 AND series IS NOT NULL ORDER BY series",
            )
            .map_err(logged(
                "reading",
                "books",
                DatabaseError::TaskPreparationFailure,
            ))?;
        let rows = stmt
            .query_map([directory_id], |row| row.get::<_, i64>(0))
            .map_err(logged("reading", "books", DatabaseError::QueryFailure))?;
        rows.collect::<Result<Vec<i64>, _>>().map_err(logged(
            "reading",
            "books",
            DatabaseError::NextRowFailure,
        ))
    }

    /// Whether the library holds the series.
    pub fn library_holds_series_with_connection(
        conn: &Connection,
        library_id: i64,
        series_id: i64,
    ) -> Result<bool, DatabaseError> {
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM library_elements \
             WHERE library_id = ?1 AND series_id = ?2)",
            [library_id, series_id],
            |row| row.get(0),
        )
        .map_err(logged(
            "reading",
            "library_elements",
            DatabaseError::QueryFailure,
        ))
    }

    /// Looks up a row id by a single column value.
    /// Legacy: new code should not use this; it remains for the folder scanner.
    pub fn get_id_from_table(
        &self,
        db_type: DatabaseTypes,
        column: impl Into<ColumnSelector>,
        value_to_search_for: &dyn rusqlite::ToSql,
    ) -> Result<Option<i64>, DatabaseError> {
        let conn = self
            .pool
            .get()
            .map_err(|_| DatabaseError::PoolConnectionRetrievalFailure)?;
        let table_name = db_type.get_table_name();
        let column_name = match column.into() {
            ColumnSelector::Book(col) => col.as_ref().to_string(),
            ColumnSelector::Library(col) => col.as_ref().to_string(),
            ColumnSelector::Series(col) => col.as_ref().to_string(),
            ColumnSelector::User(col) => col.as_ref().to_string(),
        };
        let query = format!("SELECT id FROM {} WHERE {} = ?1", table_name, column_name);
        let mut stmt = conn.prepare(&query).map_err(logged(
            "reading",
            table_name,
            DatabaseError::TaskPreparationFailure,
        ))?;
        let mut rows = stmt
            .query(rusqlite::params![value_to_search_for])
            .map_err(logged("reading", table_name, DatabaseError::QueryFailure))?;

        if let Some(row) =
            rows.next()
                .map_err(logged("reading", table_name, DatabaseError::NextRowFailure))?
        {
            let id: i64 = row.get(0).map_err(logged(
                "reading",
                table_name,
                DatabaseError::IdRetrievalFailure,
            ))?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    /// Runs `operation` inside a single SQLite transaction. It commits if the closure
    /// returns `Ok`; on `Err` the transaction is dropped, which rolls it back, and the
    /// closure's own error is returned unchanged.
    pub fn setup_new_transaction<F, T>(&self, operation: F) -> Result<T, AppErrors>
    where
        F: FnOnce(&Connection) -> Result<T, AppErrors>,
    {
        let mut conn = self.pool.get().map_err(|err| {
            tracing::error!("Got an error when trying to set up a new connection: {err}");
            DatabaseError::PoolConnectionRetrievalFailure
        })?;
        // IMMEDIATE takes the write lock up front. A deferred transaction that reads and
        // then writes fails at once with SQLITE_BUSY_SNAPSHOT if another connection wrote
        // in between, and the busy timeout can't help it.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| {
                tracing::error!("Failed to set up a transaction: {err}");
                DatabaseError::TransactionFailure
            })?;
        let result = operation(&tx)?;
        tx.commit().map_err(|err| {
            tracing::error!("Failure while commiting the transaction: {err}");
            DatabaseError::OperationCommitFailure
        })?;
        Ok(result)
    }

    /// [`Database::setup_new_transaction`] for async handlers. SQLite calls block the
    /// thread, so they run on Tokio's blocking pool instead of an async worker thread.
    pub async fn transact<F, T>(self: Arc<Self>, operation: F) -> Result<T, AppErrors>
    where
        F: FnOnce(&Connection) -> Result<T, AppErrors> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(move || self.setup_new_transaction(operation))
            .await
            .map_err(|err| {
                tracing::error!("The transaction task did not finish: {err}");
                DatabaseError::TransactionFailure
            })?
    }

    /// Runs read-only work on a pooled connection on Tokio's blocking pool. No transaction:
    /// an `IMMEDIATE` one takes the write lock, and every request's session lookup would
    /// queue behind every write.
    pub async fn read<F, T>(self: Arc<Self>, operation: F) -> Result<T, AppErrors>
    where
        F: FnOnce(&Connection) -> Result<T, AppErrors> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let conn = self.pool.get().map_err(|err| {
                tracing::error!("Got an error when trying to set up a new connection: {err}");
                DatabaseError::PoolConnectionRetrievalFailure
            })?;
            operation(&conn)
        })
        .await
        .map_err(|err| {
            tracing::error!("The read task did not finish: {err}");
            DatabaseError::TransactionFailure
        })?
    }
}

/// Converts a `SystemTime` to whole seconds since the Unix epoch, the format every
/// timestamp column uses. Panics for times before 1970.
pub fn convert_system_time_to_unix_time(time: SystemTime) -> i64 {
    checked_unix_time(time).expect("the server's own times lie after 1970")
}

/// Reads the `(id, count)` rows of a `GROUP BY` query into a map from id to count.
fn counts_by_id(
    conn: &Connection,
    query: &str,
    params: impl rusqlite::Params,
    table: &'static str,
) -> Result<HashMap<i64, i64>, DatabaseError> {
    let mut stmt = conn.prepare(query).map_err(logged(
        "counting",
        table,
        DatabaseError::TaskPreparationFailure,
    ))?;
    let rows = stmt
        .query_map(params, |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(logged("counting", table, DatabaseError::QueryFailure))?;
    rows.collect::<Result<HashMap<i64, i64>, _>>()
        .map_err(logged("counting", table, DatabaseError::NextRowFailure))
}

/// Whole seconds since the Unix epoch, or `None` for a time before it — for times the
/// server doesn't make itself, such as a file's modification time.
pub fn checked_unix_time(time: SystemTime) -> Option<i64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|age| age.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_types::AuthenticationError;
    use crate::test_support::{capture_logs, insert_user, memory_connection, TempDatabase};

    #[test]
    fn logged_logs_what_failed_with_its_cause_and_answers_the_given_error() {
        let mut answer = None;
        let logs = capture_logs(|| {
            answer = Some(logged("reading", "books", DatabaseError::QueryFailure)(
                rusqlite::Error::QueryReturnedNoRows,
            ));
        });

        assert!(matches!(answer, Some(DatabaseError::QueryFailure)));
        assert!(
            logs.contains("reading books failed: Query returned no rows"),
            "{logs}"
        );
    }

    fn count_users(db: &Database) -> i64 {
        let conn = db.pool.get().unwrap();
        conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap()
    }

    fn foreign_keys_setting(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn test_connections_enforce_foreign_keys() {
        let conn = memory_connection();
        assert_eq!(foreign_keys_setting(&conn), 1);
        let orphan = conn.execute(
            "INSERT INTO totp (user_id, authentication_secret) VALUES (999, 'secret')",
            [],
        );
        assert!(orphan.is_err(), "a row for a missing user must be refused");
    }

    #[test]
    fn pooled_connections_enforce_foreign_keys() {
        let temp = TempDatabase::open();
        let conn = temp.db.pool.get().unwrap();
        assert_eq!(foreign_keys_setting(&conn), 1);
    }

    #[test]
    fn failed_transaction_rolls_back_and_returns_its_own_error() {
        let temp = TempDatabase::open();
        let result: Result<(), AppErrors> = temp.db.setup_new_transaction(|conn| {
            insert_user(conn, "alice");
            Err(AuthenticationError::IncorrectCredentials.into())
        });

        assert!(matches!(
            result,
            Err(AppErrors::Authentication(
                AuthenticationError::IncorrectCredentials
            ))
        ));
        assert_eq!(count_users(&temp.db), 0);
    }

    #[test]
    fn transactions_take_the_write_lock_before_their_first_statement() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|_conn| {
                let other = Connection::open(temp.path()).unwrap();
                other.busy_timeout(std::time::Duration::ZERO).unwrap();

                let blocked = other.execute_batch("BEGIN IMMEDIATE; COMMIT;");
                assert!(
                    matches!(
                        blocked,
                        Err(rusqlite::Error::SqliteFailure(ref e, _))
                            if e.code == rusqlite::ErrorCode::DatabaseBusy
                    ),
                    "another writer must already be locked out, got {blocked:?}"
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn successful_transaction_commits() {
        let temp = TempDatabase::open();
        let id = temp
            .db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();

        assert!(id > 0);
        assert_eq!(count_users(&temp.db), 1);
    }

    #[tokio::test]
    async fn transact_commits_on_ok_and_returns_errors_unchanged() {
        let temp = TempDatabase::open();

        let id = temp
            .db
            .clone()
            .transact(|conn| Ok(insert_user(conn, "alice")))
            .await
            .unwrap();
        assert!(id > 0);

        let failed: Result<(), AppErrors> = temp
            .db
            .clone()
            .transact(|conn| {
                insert_user(conn, "bob");
                Err(AuthenticationError::IncorrectCredentials.into())
            })
            .await;
        assert!(matches!(
            failed,
            Err(AppErrors::Authentication(
                AuthenticationError::IncorrectCredentials
            ))
        ));

        assert_eq!(count_users(&temp.db), 1);
    }

    #[test]
    fn remove_entry_deletes_by_the_tables_key_column() {
        let conn = memory_connection();
        let user_id = insert_user(&conn, "alice");
        conn.execute(
            "INSERT INTO totp (user_id, authentication_secret) VALUES (?1, 'secret')",
            [user_id],
        )
        .unwrap();

        let removed =
            Database::remove_entry_with_connection(&conn, DatabaseTypes::Totp, user_id).unwrap();
        assert_eq!(removed, 1);

        let removed_again =
            Database::remove_entry_with_connection(&conn, DatabaseTypes::Totp, user_id).unwrap();
        assert_eq!(removed_again, 0);
    }

    #[test]
    fn a_time_before_1970_has_no_unix_time() {
        let epoch = std::time::UNIX_EPOCH;
        assert_eq!(checked_unix_time(epoch), Some(0));
        assert_eq!(
            checked_unix_time(epoch + std::time::Duration::from_secs(5)),
            Some(5)
        );
        assert_eq!(
            checked_unix_time(epoch - std::time::Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn remove_entry_refuses_the_junction_tables() {
        let conn = memory_connection();
        for junction in [
            DatabaseTypes::LibraryElements,
            DatabaseTypes::Translations,
            DatabaseTypes::ReadingModes,
        ] {
            let result = Database::remove_entry_with_connection(&conn, junction.clone(), 1);
            assert!(
                matches!(result, Err(DatabaseError::NoSingleKeyColumn)),
                "{junction:?}"
            );
        }
    }

    fn insert_series(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    /// A book of `series`, written the way the scanner writes one.
    fn insert_book(
        conn: &Connection,
        series: i64,
        title: &str,
        volume: Option<i64>,
        chapter: Option<i64>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path, \
             volume_number, chapter_number, series) \
             VALUES (?1, 'Unknown Author', 'cbz', 'unknown', '', ?2, ?3, ?4, ?5)",
            rusqlite::params![
                title,
                format!("/books/{}.cbz", uuid::Uuid::new_v4()),
                volume,
                chapter,
                series
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn a_series_books_come_in_reading_order() {
        let conn = memory_connection();
        let series = insert_series(&conn, "the long road");
        let other = insert_series(&conn, "another");
        // Written out of order, so the order has to come from the query.
        let extras = insert_book(&conn, series, "Extras", None, None);
        let volume_two = insert_book(&conn, series, "The Long Road", Some(2), None);
        let chapter_three = insert_book(&conn, series, "The Long Road", None, Some(3));
        let bonus = insert_book(&conn, series, "bonus", None, None);
        let one_two = insert_book(&conn, series, "The Long Road", Some(1), Some(2));
        let one_without_chapter = insert_book(&conn, series, "The Long Road", Some(1), None);
        let chapter_one = insert_book(&conn, series, "The Long Road", None, Some(1));
        let one_one = insert_book(&conn, series, "The Long Road", Some(1), Some(1));
        insert_book(&conn, other, "Elsewhere", Some(1), None);

        let ids = Database::get_books_in_series_with_connection(&conn, series).unwrap();

        // "bonus" before "Extras": a plain byte order would put the capital E first.
        assert_eq!(
            ids,
            [
                one_one,
                one_two,
                one_without_chapter,
                volume_two,
                chapter_one,
                chapter_three,
                bonus,
                extras
            ]
        );
    }

    #[test]
    fn a_librarys_series_come_by_name_ignoring_case() {
        let conn = memory_connection();
        let library = |name: &str| {
            conn.execute(
                "INSERT INTO library (library_name, library_type) VALUES (?1, 'books')",
                [name],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let (books, other) = (library("books"), library("other"));
        let zeta = insert_series(&conn, "zeta");
        let beta = insert_series(&conn, "Beta");
        let alpha = insert_series(&conn, "alpha");
        let elsewhere = insert_series(&conn, "aardvark");
        for (library_id, series_id) in [
            (books, zeta),
            (books, beta),
            (books, alpha),
            (other, elsewhere),
        ] {
            conn.execute(
                "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                [library_id, series_id],
            )
            .unwrap();
        }

        let ids = Database::get_series_entries_in_library_with_connection(&conn, books).unwrap();

        // A plain byte order would put "Beta" first.
        assert_eq!(ids, [alpha, beta, zeta]);
    }

    fn insert_library(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO library (library_name, library_type) VALUES (?1, 'books')",
            [name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn link(conn: &Connection, library: i64, series: i64) {
        conn.execute(
            "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
            [library, series],
        )
        .unwrap();
    }

    #[test]
    fn each_librarys_series_are_counted() {
        let conn = memory_connection();
        let manga = insert_library(&conn, "manga");
        let comics = insert_library(&conn, "comics");
        let empty = insert_library(&conn, "empty");
        let (one, two, shared) = (
            insert_series(&conn, "one"),
            insert_series(&conn, "two"),
            insert_series(&conn, "shared"),
        );
        for (library, series) in [
            (manga, one),
            (manga, two),
            (manga, shared),
            (comics, shared),
        ] {
            link(&conn, library, series);
        }

        let counts = Database::count_series_per_library_with_connection(&conn).unwrap();

        assert_eq!(counts, HashMap::from([(manga, 3), (comics, 1)]));
        assert_eq!(
            counts.get(&empty),
            None,
            "a library without series is absent"
        );
    }

    #[test]
    fn a_librarys_series_count_only_their_own_books() {
        let conn = memory_connection();
        let manga = insert_library(&conn, "manga");
        let comics = insert_library(&conn, "comics");
        let alpha = insert_series(&conn, "alpha");
        let beta = insert_series(&conn, "beta");
        let empty = insert_series(&conn, "empty");
        let elsewhere = insert_series(&conn, "elsewhere");
        for (library, series) in [
            (manga, alpha),
            (manga, beta),
            (manga, empty),
            (comics, elsewhere),
        ] {
            link(&conn, library, series);
        }
        for volume in 1..=3 {
            insert_book(&conn, alpha, "Alpha", Some(volume), None);
        }
        insert_book(&conn, beta, "Beta", Some(1), None);
        insert_book(&conn, elsewhere, "Elsewhere", Some(1), None);
        conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path) \
             VALUES ('Loose', 'Unknown Author', 'cbz', 'unknown', '', '/books/loose.cbz')",
            [],
        )
        .unwrap();

        let counts =
            Database::count_books_per_series_in_library_with_connection(&conn, manga).unwrap();

        assert_eq!(counts, HashMap::from([(alpha, 3), (beta, 1)]));
        assert_eq!(counts.get(&empty), None, "a series without books is absent");
    }

    #[test]
    fn every_series_comes_by_name_ignoring_case() {
        let conn = memory_connection();
        let zeta = insert_series(&conn, "zeta");
        let beta = insert_series(&conn, "Beta");
        let alpha = insert_series(&conn, "alpha");

        let ids = Database::all_series_ids_by_name_with_connection(&conn).unwrap();

        // A plain byte order would put "Beta" first.
        assert_eq!(ids, [alpha, beta, zeta]);
    }

    #[test]
    fn each_series_counts_its_books_in_every_library() {
        let conn = memory_connection();
        let manga = insert_library(&conn, "manga");
        let alpha = insert_series(&conn, "alpha");
        let unlinked = insert_series(&conn, "unlinked");
        let empty = insert_series(&conn, "empty");
        link(&conn, manga, alpha);
        for volume in 1..=2 {
            insert_book(&conn, alpha, "Alpha", Some(volume), None);
        }
        insert_book(&conn, unlinked, "Unlinked", Some(1), None);
        conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path) \
             VALUES ('Loose', 'Unknown Author', 'cbz', 'unknown', '', '/books/loose.cbz')",
            [],
        )
        .unwrap();

        let counts = Database::count_books_per_series_with_connection(&conn).unwrap();

        assert_eq!(counts, HashMap::from([(alpha, 2), (unlinked, 1)]));
        assert_eq!(counts.get(&empty), None, "a series without books is absent");
    }

    #[test]
    fn each_series_lists_its_libraries_and_one_in_none_is_absent() {
        let conn = memory_connection();
        let (manga, favourites) = (
            insert_library(&conn, "manga"),
            insert_library(&conn, "favourites"),
        );
        let (shared, single, alone) = (
            insert_series(&conn, "shared"),
            insert_series(&conn, "single"),
            insert_series(&conn, "alone"),
        );
        for (library, series) in [(favourites, shared), (manga, shared), (manga, single)] {
            link(&conn, library, series);
        }

        let libraries = Database::libraries_per_series_with_connection(&conn).unwrap();

        assert_eq!(
            libraries,
            HashMap::from([(shared, vec![manga, favourites]), (single, vec![manga])])
        );
        assert_eq!(libraries.get(&alone), None);
    }

    #[test]
    fn a_library_holds_only_the_series_linked_to_it() {
        let conn = memory_connection();
        let (manga, comics) = (
            insert_library(&conn, "manga"),
            insert_library(&conn, "comics"),
        );
        let berserk = insert_series(&conn, "berserk");
        link(&conn, manga, berserk);

        let holds = |library: i64, series: i64| {
            Database::library_holds_series_with_connection(&conn, library, series).unwrap()
        };
        assert!(holds(manga, berserk));
        assert!(!holds(comics, berserk));
        assert!(!holds(manga, berserk + 1));
    }

    #[test]
    fn a_library_without_series_has_no_book_counts() {
        let conn = memory_connection();
        let empty = insert_library(&conn, "empty");
        let elsewhere = insert_series(&conn, "elsewhere");
        insert_book(&conn, elsewhere, "Elsewhere", Some(1), None);

        let counts =
            Database::count_books_per_series_in_library_with_connection(&conn, empty).unwrap();

        assert!(counts.is_empty());
    }

    fn insert_directory(conn: &Connection, path: &str) -> i64 {
        conn.execute("INSERT INTO directories (path) VALUES (?1)", [path])
            .unwrap();
        conn.last_insert_rowid()
    }

    fn place(conn: &Connection, book: i64, directory: i64) {
        conn.execute(
            "UPDATE books SET directory_id = ?1 WHERE id = ?2",
            [directory, book],
        )
        .unwrap();
    }

    #[test]
    fn only_the_books_in_no_directory_are_listed_with_their_stored_paths() {
        let conn = memory_connection();
        let berserk = insert_series(&conn, "berserk");
        let books = insert_directory(&conn, "/books");
        let placed = insert_book(&conn, berserk, "Berserk", Some(1), None);
        place(&conn, placed, books);
        let first = insert_book(&conn, berserk, "Berserk", Some(2), None);
        let second = insert_book(&conn, berserk, "Berserk", Some(3), None);
        let path_of = |id: i64| -> String {
            conn.query_row("SELECT file_path FROM books WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
        };

        let unplaced = Database::books_without_directory_with_connection(&conn).unwrap();

        assert_eq!(
            unplaced,
            [(first, path_of(first)), (second, path_of(second))]
        );
    }

    #[test]
    fn a_directorys_series_are_those_holding_one_of_its_books_each_once() {
        let conn = memory_connection();
        let manga = insert_directory(&conn, "/manga");
        let comics = insert_directory(&conn, "/comics");
        let berserk = insert_series(&conn, "berserk");
        let monster = insert_series(&conn, "monster");
        let saga = insert_series(&conn, "saga");
        for (series, directory) in [(monster, manga), (berserk, manga), (berserk, manga)] {
            let book = insert_book(&conn, series, "Volume", None, None);
            place(&conn, book, directory);
        }
        let other = insert_book(&conn, saga, "Saga", None, None);
        place(&conn, other, comics);
        insert_book(&conn, saga, "Saga", Some(2), None);

        assert_eq!(
            Database::series_of_directory_with_connection(&conn, manga).unwrap(),
            [berserk, monster]
        );
        assert_eq!(
            Database::series_of_directory_with_connection(&conn, manga + 99).unwrap(),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn a_series_is_found_by_its_match_key_only_where_it_holds_a_book_of_the_directory() {
        let conn = memory_connection();
        let manga = insert_directory(&conn, "/manga");
        let comics = insert_directory(&conn, "/comics");
        let keyed = |name: &str| -> i64 {
            conn.execute(
                "INSERT INTO series (name, match_key) VALUES (?1, lower(?1))",
                [name],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let berserk_in_manga = keyed("Berserk");
        let berserk_in_comics = keyed("Berserk");
        let younger_in_manga = keyed("BERSERK");
        keyed("Pluto");
        let unplaced = keyed("Monster");
        for (series, directory) in [
            (berserk_in_manga, manga),
            (berserk_in_comics, comics),
            (younger_in_manga, manga),
        ] {
            let book = insert_book(&conn, series, "Berserk", None, None);
            place(&conn, book, directory);
        }
        insert_book(&conn, unplaced, "Monster", None, None);
        let find = |directory: i64, key: &str| {
            Database::series_of_directory_by_match_key_with_connection(&conn, directory, key)
                .unwrap()
        };

        assert_eq!(find(manga, "berserk"), Some(berserk_in_manga), "the oldest");
        assert_eq!(find(comics, "berserk"), Some(berserk_in_comics));
        assert_eq!(find(manga, "Berserk"), None, "the caller lowercases");
        assert_eq!(find(manga, "pluto"), None, "a series without books");
        assert_eq!(
            find(manga, "monster"),
            None,
            "its book lies in no directory"
        );
    }

    #[test]
    fn a_database_knows_its_folder_with_every_symlink_resolved() {
        let temp = TempDatabase::open();

        let folder = temp.db.folder();

        assert!(folder.is_absolute());
        assert_eq!(
            folder,
            std::fs::canonicalize(temp.path().parent().unwrap()).unwrap()
        );
    }
}
