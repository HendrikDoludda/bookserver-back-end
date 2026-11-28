//! The metadata handlers: looking a series up online.

use std::{sync::Arc, time::SystemTime};

use axum::{extract::State, Json};

use crate::{
    data_models::metadata_model::{CandidatesQuery, MetadataCandidates},
    db::Database,
    error_types::AppErrors,
    metadata::MetadataProviders,
    routes::metadata_lookup,
};

use super::extractors::{AdminSession, PathParams, QueryParams};

/// `GET /api/v1/series/{id}/metadata/candidates?query=`: the candidates the series' provider finds
/// for `query` — by default the series' name — each marked `exact_match` when one of its titles is
/// exactly the query. 409 `sensitive_item` or `no_metadata_provider`, 400 `invalid_query`, 503
/// `provider_rate_limited` with `Retry-After`, 502 `provider_unavailable`.
pub async fn get_metadata_candidates(
    _admin: AdminSession,
    State(db): State<Arc<Database>>,
    State(providers): State<MetadataProviders>,
    PathParams(series_id): PathParams<i64>,
    QueryParams(query): QueryParams<CandidatesQuery>,
) -> Result<Json<MetadataCandidates>, AppErrors> {
    Ok(Json(
        metadata_lookup::candidates(db, &providers, series_id, query.query, SystemTime::now())
            .await?,
    ))
}
