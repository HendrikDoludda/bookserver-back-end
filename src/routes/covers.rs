//! Which cover file a book, a series or a library shows.

use std::sync::Arc;

use rusqlite::Connection;
use serde::Deserialize;

use crate::{
    db::Database,
    error_types::{AppErrors, DatabaseError},
    models::{BookMetadata, BookSeriesMetadata, LibraryMetadata},
};

/// The `{kind}` of `GET /api/v1/covers/{kind}/{id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverKind {
    Book,
    Series,
    Library,
}

/// The stored path of the cover the `kind` row `id` shows, or `None` when it has none. A row that
/// doesn't exist is `DatabaseError::EntryNotFound`.
pub async fn cover_path(
    db: Arc<Database>,
    kind: CoverKind,
    id: i64,
) -> Result<Option<String>, AppErrors> {
    db.read(move |conn| {
        Ok(match kind {
            CoverKind::Book => {
                Database::get_entry_with_connection::<BookMetadata>(conn, id)?.cover_image
            }
            CoverKind::Series => series_cover(conn, id)?,
            CoverKind::Library => library_cover(conn, id)?,
        })
    })
    .await
}

// The fallback is worked out at every read and never stored: a stored one couldn't be told apart
// from a cover a user chose, so a re-scan could never safely replace it.
fn series_cover(conn: &Connection, series_id: i64) -> Result<Option<String>, DatabaseError> {
    let series = Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
    match series.cover_image {
        Some(own) => Ok(Some(own)),
        None => Database::first_book_cover_with_connection(conn, series_id),
    }
}

fn library_cover(conn: &Connection, library_id: i64) -> Result<Option<String>, DatabaseError> {
    let library = Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
    if library.cover_image.is_some() {
        return Ok(library.cover_image);
    }
    for series_id in Database::get_series_entries_in_library_with_connection(conn, library_id)? {
        if let Some(cover) = series_cover(conn, series_id)? {
            return Ok(Some(cover));
        }
    }
    Ok(None)
}
