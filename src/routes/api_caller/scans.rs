//! Scanning the added folders, all of them or one, in the background, and the scan's status.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};

use crate::{
    data_models::scan_model::{ScanScope, ScanStatus},
    db::Database,
    error_types::AppErrors,
    models::Directory,
    scanner::scan_jobs::ScanJobs,
};

use super::extractors::{AdminSession, PathParams};

/// `POST /api/v1/scan`: scans every added folder in the background. Answers 202 and the status
/// that shows the scan waiting, or 409 `scan_in_progress` while a scan runs or waits.
pub async fn scan_all_directories(
    _admin: AdminSession,
    State(scans): State<ScanJobs>,
) -> Result<(StatusCode, Json<ScanStatus>), AppErrors> {
    Ok((StatusCode::ACCEPTED, Json(scans.request(ScanScope::All)?)))
}

/// `POST /api/v1/directories/{id}/scan`: scans one added folder in the background; answers as
/// `POST /api/v1/scan` does, and 404 for a folder that isn't added.
pub async fn scan_directory(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    State(scans): State<ScanJobs>,
    PathParams(id): PathParams<i64>,
) -> Result<(StatusCode, Json<ScanStatus>), AppErrors> {
    db.read(move |conn| Ok(Database::get_entry_with_connection::<Directory>(conn, id)?))
        .await?;
    let status = scans.request(ScanScope::directory(id))?;
    Ok((StatusCode::ACCEPTED, Json(status)))
}

/// `GET /api/v1/scan/status`: whether a scan runs and how far it got, the scan that waits, and
/// how the last one ended.
pub async fn scan_status(_admin: AdminSession, State(scans): State<ScanJobs>) -> Json<ScanStatus> {
    Json(scans.status())
}
