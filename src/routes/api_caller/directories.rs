use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};

use crate::{
    data_models::books_model::{DirectorySummary, NewDirectory},
    db::Database,
    error_types::AppErrors,
    models::WithId,
    routes::directory_management,
    scanner::cover_image_retriever::Covers,
};

use super::extractors::{AdminSession, JsonBody, PathParams};

/// `GET /api/v1/directories`: every scanned folder by path, with the library its series join.
pub async fn list_directories(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
) -> Result<Json<Vec<WithId<DirectorySummary>>>, AppErrors> {
    Ok(Json(directory_management::all_directories(db).await?))
}

/// `POST /api/v1/directories` with `{"path", "create_library", "library_type"?,
/// "library_name"?}`: answers 201 and the folder, its path as stored.
pub async fn add_directory(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    State(covers): State<Covers>,
    JsonBody(new): JsonBody<NewDirectory>,
) -> Result<(StatusCode, Json<WithId<DirectorySummary>>), AppErrors> {
    let directory = directory_management::add_directory(db, covers.folder, new).await?;
    Ok((StatusCode::CREATED, Json(directory)))
}

/// `DELETE /api/v1/directories/{id}`: its books leave the catalog, and the series left without
/// books with them; the files stay.
pub async fn remove_directory(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    PathParams(id): PathParams<i64>,
) -> Result<StatusCode, AppErrors> {
    directory_management::remove_directory(db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
