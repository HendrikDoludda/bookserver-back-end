//! What an admin looks up online about a series: the candidates its provider finds.

use std::{sync::Arc, time::SystemTime};

use crate::{
    data_models::metadata_model::{CandidateMatch, MetadataCandidates},
    db::Database,
    error_types::{AppErrors, MetadataError},
    metadata::{clear, Caller, MetadataProviders},
};

/// As long as a series' name may be (`checked_name`), so a name is always a search.
const MAX_QUERY_CHARS: usize = 200;

/// What the series' provider finds for `query` — by default the series' name — each candidate
/// marked when one of its titles is exactly the query. Changes no series; the answer is kept for
/// the next lookup.
pub async fn candidates(
    db: Arc<Database>,
    providers: &MetadataProviders,
    series_id: i64,
    query: Option<String>,
    now: SystemTime,
) -> Result<MetadataCandidates, AppErrors> {
    let series = Arc::clone(&db)
        .read(move |conn| clear(conn, series_id))
        .await?;
    let query = checked_query(query.as_deref().unwrap_or(series.name()))?;
    let found = providers
        .search(&db, &series, &query, Caller::Admin, now)
        .await?;
    Ok(MetadataCandidates {
        provider: series.provider().as_str(),
        candidates: found
            .into_iter()
            .map(|candidate| CandidateMatch {
                exact_match: candidate.is_exact_match(&query),
                candidate,
            })
            .collect(),
        query,
    })
}

/// A search as it is sent: trimmed, and 1 to 200 characters long.
fn checked_query(query: &str) -> Result<String, MetadataError> {
    let query = query.trim();
    if (1..=MAX_QUERY_CHARS).contains(&query.chars().count()) {
        Ok(query.to_string())
    } else {
        Err(MetadataError::InvalidQuery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_trimmed_and_has_1_to_200_characters() {
        assert_eq!(checked_query("  Berserk \n").unwrap(), "Berserk");
        for refused in [String::new(), "  \t".to_string(), "x".repeat(201)] {
            assert!(
                matches!(checked_query(&refused), Err(MetadataError::InvalidQuery)),
                "{refused:?}"
            );
        }
        // Characters, not bytes: 200 kana are 600 bytes.
        for accepted in ["x".repeat(200), "ベ".repeat(200)] {
            assert_eq!(checked_query(&accepted).unwrap(), accepted);
        }
    }
}
