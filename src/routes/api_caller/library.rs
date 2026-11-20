//! TODO: needs a database table of scannable directories (replacing `BOOK_DIRS`),
//!       with a link to the metadata-agent result.
//! TODO: check that served file paths lie inside a scannable directory or the
//!       cover-images folder.

use std::{path::Path, sync::Arc};

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, Response, StatusCode},
    response::IntoResponse,
    Json,
};

use crate::{
    book_pages::{self, PageContent},
    data_models::books_model::{
        BookDetails, BookSummary, LibrarySummary, PageSummary, ReadingModeChange,
        ReadingModeSetting, SeriesSummary,
    },
    db::Database,
    error_types::{internal_error_response, AppErrors, DatabaseError, PageError},
    folder_scanner::scan_all_folders,
    models::{BookMetadata, BookSeriesMetadata, DatabaseTypes, LibraryMetadata, WithId},
    routes::{
        covers::{self, CoverKind},
        reading_modes,
    },
    stream_reader::serve_file,
};

use super::extractors::{AdminSession, AuthSession, JsonBody, PathParams};

// === Library, series & book handlers =========================================

/// Return every library with its number of series, each paired with its row id.
pub async fn get_libraries(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
) -> Result<Json<Vec<WithId<LibrarySummary>>>, AppErrors> {
    let libraries = db
        .read(|conn| {
            let libraries = Database::get_all_entries_with_connection::<LibraryMetadata>(conn)?;
            let series_counts = Database::count_series_per_library_with_connection(conn)?;
            Ok(libraries
                .into_iter()
                .map(|library| WithId {
                    id: library.id,
                    data: LibrarySummary {
                        series_count: series_counts.get(&library.id).copied().unwrap_or(0),
                        library: library.data,
                    },
                })
                .collect::<Vec<_>>())
        })
        .await?;
    Ok(Json(libraries))
}

/// Return every series that belongs to the given library, by name, with its number of books,
/// each with its row id.
pub async fn get_series_in_library(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(library_id): PathParams<i64>,
) -> Result<Json<Vec<WithId<SeriesSummary>>>, AppErrors> {
    let series = db
        .read(move |conn| {
            let series_ids =
                Database::get_series_entries_in_library_with_connection(conn, library_id)?;
            let series = Database::get_entries_batch_with_connection::<BookSeriesMetadata>(
                conn,
                &series_ids,
            )?;
            let book_counts =
                Database::count_books_per_series_in_library_with_connection(conn, library_id)?;
            Ok(series
                .into_iter()
                .map(|one| WithId {
                    id: one.id,
                    data: SeriesSummary {
                        book_count: book_counts.get(&one.id).copied().unwrap_or(0),
                        series: one.data,
                    },
                })
                .collect::<Vec<_>>())
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
    // TODO: also serve partial books.
    let book = read_book(db, id).await?;
    Ok(serve_file(Path::new(&book.file_path), request).await)
}

/// List the pages of an image-folder or CBZ book, numbered from 1, each with its size.
pub async fn get_pages(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<Json<Vec<PageSummary>>, AppErrors> {
    let book = read_book(db, id).await?;
    let sizes =
        blocking(move || book_pages::page_sizes(&book.format, Path::new(&book.file_path))).await?;
    Ok(Json(
        sizes
            .into_iter()
            .zip(1..)
            .map(|(size, number)| PageSummary {
                number,
                width: size.map(|size| size.width),
                height: size.map(|size| size.height),
            })
            .collect(),
    ))
}

/// Serve one page of an image-folder or CBZ book; pages are numbered from 1.
pub async fn get_page(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams((id, number)): PathParams<(i64, usize)>,
    request: Request<Body>,
) -> Result<Response<Body>, AppErrors> {
    let book = read_book(db, id).await?;
    let page = blocking(move || {
        book_pages::page(
            &book.format,
            Path::new(&book.file_path),
            number,
            book_pages::MAX_PAGE_BYTES,
        )
    })
    .await?;
    Ok(match page {
        PageContent::File(path) => serve_file(&path, request).await,
        PageContent::Bytes { bytes, media_type } => {
            ([(header::CONTENT_TYPE, media_type)], bytes).into_response()
        }
    })
}

/// Return a single book's metadata, and the mode the calling user opens it in, without streaming
/// the file itself.
pub async fn retrieve_metadata(
    session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<Json<BookDetails>, AppErrors> {
    // TODO: return metadata for every metadata type, not just books.
    let user_id = session.0.user_id;
    let (book, reading_mode) = db
        .read(move |conn| {
            let book = Database::get_entry_with_connection::<BookMetadata>(conn, id)?;
            let reading_mode = reading_modes::mode_for(conn, user_id, &book)?;
            Ok((book, reading_mode))
        })
        .await?;
    Ok(Json(BookDetails {
        book: BookSummary::from(book),
        reading_mode,
    }))
}

/// `PUT /api/v1/series/{id}/reading_mode`: the calling user's mode for every book of the series;
/// answers it as stored.
pub async fn set_reading_mode(
    session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(series_id): PathParams<i64>,
    JsonBody(change): JsonBody<ReadingModeChange>,
) -> Result<Json<ReadingModeSetting>, AppErrors> {
    let mode = reading_modes::set_mode(db, session.0.user_id, series_id, &change.mode).await?;
    Ok(Json(ReadingModeSetting { mode }))
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

/// Runs a book's file or archive reading on Tokio's blocking pool.
async fn blocking<T, F>(work: F) -> Result<T, PageError>
where
    F: FnOnce() -> Result<T, PageError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| PageError::TaskFailed)?
}
