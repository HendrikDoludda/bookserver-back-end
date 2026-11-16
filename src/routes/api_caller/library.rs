//! TODO: needs a database table of scannable directories (replacing `BOOK_DIRS`),
//!       with a link to the metadata-agent result.
//! TODO: check that served file paths lie inside a scannable directory or the
//!       cover-images folder.

use std::{path::Path, sync::Arc};

use axum::{
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    Json,
};

use crate::{
    data_models::books_model::BookSummary,
    db::Database,
    error_types::{internal_error_response, AppErrors, DatabaseError},
    folder_scanner::scan_all_folders,
    models::{BookMetadata, BookSeriesMetadata, DatabaseTypes, LibraryMetadata, WithId},
    routes::covers::{self, CoverKind},
    stream_reader::serve_file,
};

use super::extractors::{AdminSession, AuthSession, PathParams};

// === Library, series & book handlers =========================================

/// Return every library, each paired with its row id.
pub async fn get_libraries(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
) -> Result<Json<Vec<WithId<LibraryMetadata>>>, AppErrors> {
    let libraries = db
        .read(|conn| Ok(Database::get_all_entries_with_connection::<LibraryMetadata>(conn)?))
        .await?;
    Ok(Json(libraries))
}

/// Return every series that belongs to the given library, by name, each with its row id.
pub async fn get_series_in_library(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(library_id): PathParams<i64>,
) -> Result<Json<Vec<WithId<BookSeriesMetadata>>>, AppErrors> {
    let series = db
        .read(move |conn| {
            let series_ids =
                Database::get_series_entries_in_library_with_connection(conn, library_id)?;
            Ok(Database::get_entries_batch_with_connection::<
                BookSeriesMetadata,
            >(conn, &series_ids)?)
        })
        .await?;
    Ok(Json(series))
}

/// Return every book that belongs to the given series, in reading order, each with its row id.
pub async fn get_series_children(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(series_id): PathParams<i64>,
) -> Result<Json<Vec<WithId<BookSummary>>>, AppErrors> {
    let books = db
        .read(move |conn| {
            let book_ids = Database::get_books_in_series_with_connection(conn, series_id)?;
            Ok(Database::get_entries_batch_with_connection::<BookMetadata>(
                conn, &book_ids,
            )?)
        })
        .await?;

    Ok(Json(
        books
            .into_iter()
            .map(|book| WithId {
                id: book.id,
                data: BookSummary::from(book.data),
            })
            .collect(),
    ))
}

/// Stream a book's file, with HTTP range support.
pub async fn request_file(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    request: Request<Body>,
) -> Result<Response<Body>, AppErrors> {
    // TODO: also serve partial books, and folders of images (e.g. an unzipped CBZ). Image
    // comics will likely need page heights pre-calculated so the reader can lay them out
    // correctly.
    let book = read_book(db, id).await?;
    Ok(serve_file(Path::new(&book.file_path), request).await)
}

/// Return a single book's metadata without streaming the file itself.
pub async fn retrieve_metadata(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<Json<BookSummary>, AppErrors> {
    // TODO: return metadata for every metadata type, not just books.
    let book = read_book(db, id).await?;
    Ok(Json(BookSummary::from(book)))
}

/// Serve the cover a book, a series or a library shows: `kind` is `book`, `series` or `library`.
pub async fn get_cover(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams((kind, id)): PathParams<(CoverKind, i64)>,
    request: Request<Body>,
) -> Result<Response<Body>, AppErrors> {
    let path = covers::cover_path(db, kind, id)
        .await?
        .ok_or(DatabaseError::EntryNotFound)?;
    Ok(serve_file(Path::new(&path), request).await)
}

/// Kick off a scan of every configured book directory.
pub async fn scan_all_directories(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
) -> Response<Body> {
    match scan_all_folders(db).await {
        Ok(()) => (StatusCode::OK, "Directory scan complete").into_response(),
        Err(error) => {
            tracing::error!("directory scan failed: {error:#}");
            internal_error_response()
        }
    }
}

/// Delete a library row by id.
pub async fn delete_library(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<&'static str, AppErrors> {
    db.transact(move |conn| {
        Ok(Database::remove_entry_with_connection(
            conn,
            DatabaseTypes::Library,
            id,
        )?)
    })
    .await?;
    Ok("Library deleted")
}

async fn read_book(db: Arc<Database>, id: i64) -> Result<BookMetadata, AppErrors> {
    db.read(move |conn| {
        Ok(Database::get_entry_with_connection::<BookMetadata>(
            conn, id,
        )?)
    })
    .await
}
