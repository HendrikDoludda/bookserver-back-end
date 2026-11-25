use std::{sync::Arc, time::SystemTime};

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
        BookDetails, BookSummary, LibraryChanges, LibraryCoverChoice, LibrarySeries,
        LibrarySummary, NewLibrary, PageSummary, ProgressChange, ReadingModeChange,
        ReadingModeSetting, SeriesBook, SeriesChanges, SeriesCoverChoice, SeriesListing,
        SeriesMerge, SeriesSummary,
    },
    database_related_scripts::db_reading_progress,
    db::{convert_system_time_to_unix_time, Database},
    error_types::{AppErrors, DatabaseError, PageError},
    models::{
        BookMetadata, BookSeriesMetadata, DatabaseTypes, LibraryMetadata, ReadingProgress, WithId,
    },
    routes::{
        covers::CoverKind, library_management, reading_modes, reading_progress, served_files,
    },
    scanner::cover_image_retriever::Covers,
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
                    data: LibrarySummary::new(
                        library.data,
                        series_counts.get(&library.id).copied().unwrap_or(0),
                    ),
                })
                .collect::<Vec<_>>())
        })
        .await?;
    Ok(Json(libraries))
}

/// Return every series that belongs to the given library, by name, with its number of books and
/// how many of them the calling user finished, each with its row id.
pub async fn get_series_in_library(
    session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(library_id): PathParams<i64>,
) -> Result<Json<Vec<WithId<LibrarySeries>>>, AppErrors> {
    let user_id = session.0.user_id;
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
            let finished_counts =
                db_reading_progress::finished_per_series_in_library(conn, user_id, library_id)?;
            Ok(series
                .into_iter()
                .map(|one| WithId {
                    id: one.id,
                    data: LibrarySeries {
                        finished_count: finished_counts.get(&one.id).copied().unwrap_or(0),
                        series: SeriesSummary::new(
                            one.data,
                            book_counts.get(&one.id).copied().unwrap_or(0),
                        ),
                    },
                })
                .collect::<Vec<_>>())
        })
        .await?;
    Ok(Json(series))
}

/// Return every book that belongs to the given series, in reading order, with the calling user's
/// progress in it, each with its row id.
pub async fn get_series_children(
    session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(series_id): PathParams<i64>,
) -> Result<Json<Vec<WithId<SeriesBook>>>, AppErrors> {
    let user_id = session.0.user_id;
    let (books, progress) = db
        .read(move |conn| {
            let book_ids = Database::get_books_in_series_with_connection(conn, series_id)?;
            let books =
                Database::get_entries_batch_with_connection::<BookMetadata>(conn, &book_ids)?;
            let progress = db_reading_progress::of_series(conn, user_id, series_id)?;
            Ok((books, progress))
        })
        .await?;

    Ok(Json(
        books
            .into_iter()
            .map(|book| WithId {
                id: book.id,
                data: SeriesBook {
                    progress: progress.get(&book.id).copied().unwrap_or_default(),
                    book: BookSummary::from(book.data),
                },
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
    let book = served_files::book_file(db, id).await?;
    Ok(serve_file(&book.path, request).await)
}

/// List the pages of an image-folder or CBZ book, numbered from 1, each with its size.
pub async fn get_pages(
    _session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<Json<Vec<PageSummary>>, AppErrors> {
    let book = served_files::book_file(db, id).await?;
    let sizes = blocking(move || book_pages::page_sizes(&book.format, &book.path)).await?;
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
    let book = served_files::book_file(db, id).await?;
    let page = blocking(move || {
        book_pages::page(&book.format, &book.path, number, book_pages::MAX_PAGE_BYTES)
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
    let (book, reading_mode, progress) = db
        .read(move |conn| {
            let book = Database::get_entry_with_connection::<BookMetadata>(conn, id)?;
            let reading_mode = reading_modes::mode_for(conn, user_id, &book)?;
            let progress = db_reading_progress::find(conn, user_id, id)?.unwrap_or_default();
            Ok((book, reading_mode, progress))
        })
        .await?;
    Ok(Json(BookDetails {
        book: BookSummary::from(book),
        reading_mode,
        progress,
    }))
}

/// `GET /api/v1/books/{id}/progress`: the calling user's progress in the book.
pub async fn get_progress(
    session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<Json<ReadingProgress>, AppErrors> {
    Ok(Json(
        reading_progress::progress_of(db, session.0.user_id, id).await?,
    ))
}

/// `PATCH /api/v1/books/{id}/progress` with `{"position"?, "total"?, "finished"?}`: changes the
/// calling user's progress in the book and answers it as stored.
pub async fn save_progress(
    session: AuthSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    JsonBody(change): JsonBody<ProgressChange>,
) -> Result<Json<ReadingProgress>, AppErrors> {
    let now = convert_system_time_to_unix_time(SystemTime::now());
    Ok(Json(
        reading_progress::save_progress(db, session.0.user_id, id, change, now).await?,
    ))
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
    State(cover_files): State<Covers>,
    PathParams((kind, id)): PathParams<(CoverKind, i64)>,
    request: Request<Body>,
) -> Result<Response<Body>, AppErrors> {
    let path = served_files::cover_file(db, &cover_files, kind, id).await?;
    Ok(serve_file(&path, request).await)
}

/// `POST /api/v1/libraries` with `{"name", "library_type", "description"?}`: answers 201 and the
/// new library.
pub async fn add_library(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    JsonBody(new): JsonBody<NewLibrary>,
) -> Result<(StatusCode, Json<WithId<LibrarySummary>>), AppErrors> {
    let library = library_management::create_library(db, new).await?;
    Ok((StatusCode::CREATED, Json(library)))
}

/// `PATCH /api/v1/libraries/{id}` with `{"name"?, "library_type"?, "description"?}`: answers the
/// changed library.
pub async fn update_library(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    JsonBody(changes): JsonBody<LibraryChanges>,
) -> Result<Json<WithId<LibrarySummary>>, AppErrors> {
    Ok(Json(
        library_management::change_library(db, id, changes).await?,
    ))
}

/// `GET /api/v1/series`: every series by name, to choose a library's series from.
pub async fn list_series(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
) -> Result<Json<Vec<WithId<SeriesListing>>>, AppErrors> {
    Ok(Json(library_management::all_series(db).await?))
}

/// `PUT /api/v1/libraries/{id}/series/{series_id}`: the library holds the series.
pub async fn add_series_to_library(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams((library_id, series_id)): PathParams<(i64, i64)>,
) -> Result<StatusCode, AppErrors> {
    library_management::add_series(db, library_id, series_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/libraries/{id}/series/{series_id}`: the library no longer holds the series;
/// the series and its books stay.
pub async fn remove_series_from_library(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams((library_id, series_id)): PathParams<(i64, i64)>,
) -> Result<StatusCode, AppErrors> {
    library_management::remove_series(db, library_id, series_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/v1/series/{id}` with `{"name"?, "description"?, "start_release_year"?,
/// "end_release_year"?}`: answers the changed series.
pub async fn update_series(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    JsonBody(changes): JsonBody<SeriesChanges>,
) -> Result<Json<WithId<SeriesSummary>>, AppErrors> {
    Ok(Json(
        library_management::change_series(db, id, changes).await?,
    ))
}

/// `POST /api/v1/series/{id}/merges` with `{"series_id"}`: merges that series into this one and
/// answers this one as `PATCH /api/v1/series/{id}` does; 400 `invalid_request` for itself.
pub async fn merge_series(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    JsonBody(merge): JsonBody<SeriesMerge>,
) -> Result<Json<WithId<SeriesSummary>>, AppErrors> {
    Ok(Json(
        library_management::merge_series(db, id, merge.series_id).await?,
    ))
}

/// `DELETE /api/v1/series/{id}`: only a series without books; 409 `series_not_empty` otherwise.
pub async fn delete_series(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<StatusCode, AppErrors> {
    library_management::delete_series(db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/v1/series/{id}/cover` with `{"book_id"}`: the series shows that book's cover.
pub async fn choose_series_cover(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    JsonBody(choice): JsonBody<SeriesCoverChoice>,
) -> Result<StatusCode, AppErrors> {
    library_management::choose_series_cover(db, id, choice.book_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/series/{id}/cover`: the series shows its automatic cover again.
pub async fn reset_series_cover(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<StatusCode, AppErrors> {
    library_management::reset_series_cover(db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/v1/libraries/{id}/cover` with `{"series_id"}`: the library shows the cover that
/// series shows now.
pub async fn choose_library_cover(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
    JsonBody(choice): JsonBody<LibraryCoverChoice>,
) -> Result<StatusCode, AppErrors> {
    library_management::choose_library_cover(db, id, choice.series_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/libraries/{id}/cover`: the library shows its automatic cover again.
pub async fn reset_library_cover(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<StatusCode, AppErrors> {
    library_management::reset_library_cover(db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a library row by id; 404 when no library has it.
pub async fn delete_library(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<StatusCode, AppErrors> {
    let deleted = db
        .transact(move |conn| {
            Ok(Database::remove_entry_with_connection(
                conn,
                DatabaseTypes::Library,
                id,
            )?)
        })
        .await?;
    if deleted == 0 {
        return Err(DatabaseError::EntryNotFound.into());
    }
    Ok(StatusCode::NO_CONTENT)
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
