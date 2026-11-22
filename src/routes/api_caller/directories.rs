use std::{path::PathBuf, sync::Arc};

use axum::{extract::State, http::StatusCode, Json};

use crate::{
    data_models::{
        books_model::{DirectorySummary, NewDirectory},
        scan_model::{ScanScope, ScanTrigger},
    },
    db::Database,
    error_types::AppErrors,
    models::WithId,
    routes::directory_management,
    scanner::{cover_image_retriever::Covers, folder_watch::FolderWatch, scan_jobs::ScanJobs},
};

use super::extractors::{AdminSession, JsonBody, PathParams};

/// `GET /api/v1/directories`: every scanned folder by path, with the library its series join and
/// whether its changes are watched.
pub async fn list_directories(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    State(watch): State<FolderWatch>,
) -> Result<Json<Vec<WithId<DirectorySummary>>>, AppErrors> {
    let mut directories = directory_management::all_directories(db).await?;
    for directory in &mut directories {
        directory.data.watching = watch.watching(directory.id);
    }
    Ok(Json(directories))
}

/// `POST /api/v1/directories` with `{"path", "create_library", "library_type"?,
/// "library_name"?}`: answers 201 and the folder, its path as stored, starts watching it and
/// queues its first scan.
pub async fn add_directory(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    State(covers): State<Covers>,
    State(scans): State<ScanJobs>,
    State(watch): State<FolderWatch>,
    JsonBody(new): JsonBody<NewDirectory>,
) -> Result<(StatusCode, Json<WithId<DirectorySummary>>), AppErrors> {
    let mut directory = directory_management::add_directory(db, covers.folder, new).await?;
    // Watched before its first scan starts, so a file copied in meanwhile is not missed.
    directory.data.watching = watch
        .watch(directory.id, PathBuf::from(&directory.data.path))
        .await;
    scans.queue(
        ScanScope::folder(&directory.data.path),
        ScanTrigger::DirectoryAdded,
    );
    Ok((StatusCode::CREATED, Json(directory)))
}

/// `DELETE /api/v1/directories/{id}`: its books leave the catalog, and the series left without
/// books with them; the files stay, and are no longer watched.
pub async fn remove_directory(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    State(watch): State<FolderWatch>,
    PathParams(id): PathParams<i64>,
) -> Result<StatusCode, AppErrors> {
    directory_management::remove_directory(db, id).await?;
    watch.unwatch(id).await;
    Ok(StatusCode::NO_CONTENT)
}
