//! The bodies of the metadata routes.

use serde::{Deserialize, Serialize};

use crate::metadata::Candidate;

/// The query of `GET /api/v1/series/{id}/metadata/candidates`.
#[derive(Debug, Deserialize)]
pub struct CandidatesQuery {
    /// What to search for; absent means the series' name.
    pub query: Option<String>,
}

/// `GET /api/v1/series/{id}/metadata/candidates`: the provider asked, what it was asked for, and
/// what it found, best first.
#[derive(Debug, Serialize)]
pub struct MetadataCandidates {
    pub provider: &'static str,
    pub query: String,
    pub candidates: Vec<CandidateMatch>,
}

/// One candidate, and whether one of its titles is exactly what was searched for.
#[derive(Debug, Serialize)]
pub struct CandidateMatch {
    #[serde(flatten)]
    pub candidate: Candidate,
    pub exact_match: bool,
}
