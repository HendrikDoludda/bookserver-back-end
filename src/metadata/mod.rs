//! Metadata from online providers: which provider describes a series, the gate every lookup
//! passes, and the lookups themselves.

mod anilist;
mod http;
mod plain_text;
mod rate_limit;

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};

use rusqlite::Connection;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    database_related_scripts::db_metadata,
    db::{convert_system_time_to_unix_time, Database},
    error_types::{AppErrors, MetadataError},
    models::{BookSeriesMetadata, LibraryType},
    scanner::folder_scanner::match_key,
};
use http::Endpoint;

/// An admin's request answers within this, waited or not; an app gives up not much later.
const ADMIN_WAIT: Duration = Duration::from_secs(10);

/// An online source of series metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataProvider {
    AniList,
}

impl MetadataProvider {
    /// The name it is stored and sent under.
    pub fn as_str(&self) -> &'static str {
        match self {
            MetadataProvider::AniList => "anilist",
        }
    }
}

/// The provider that describes the series of a library type; `None` for a type none covers yet.
pub fn provider_for(library_type: LibraryType) -> Option<MetadataProvider> {
    match library_type {
        LibraryType::Manga => Some(MetadataProvider::AniList),
        LibraryType::Books
        | LibraryType::Comics
        | LibraryType::Magazines
        | LibraryType::Documents
        | LibraryType::Others => None,
    }
}

/// A series that may be looked up online, and the provider that describes it. Only [`clear`]
/// makes one, so a lookup that takes it can't skip its checks.
#[derive(Debug, Clone)]
pub struct ClearedSeries {
    series_id: i64,
    name: String,
    provider: MetadataProvider,
    includes_adult: bool,
}

impl ClearedSeries {
    pub fn series_id(&self) -> i64 {
        self.series_id
    }

    /// The series' name as an admin sees it, which a search looks for unless told otherwise.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn provider(&self) -> MetadataProvider {
        self.provider
    }

    /// Whether a lookup may answer entries its provider marks adult: one of the series' libraries
    /// has `adult_metadata` on.
    pub fn includes_adult(&self) -> bool {
        self.includes_adult
    }
}

/// The series `series_id`, cleared for an online lookup. Refused when it is sensitive — by its
/// own flag or that of a library it is in, and nothing overrides that — or when none of its
/// libraries is of a type a provider covers.
pub fn clear(conn: &Connection, series_id: i64) -> Result<ClearedSeries, AppErrors> {
    let series = Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
    if Database::series_is_sensitive_with_connection(conn, series_id)? {
        return Err(MetadataError::SensitiveItem.into());
    }
    // The first one: with a single provider so far, two libraries can't name different ones.
    let provider = Database::library_types_of_series_with_connection(conn, series_id)?
        .into_iter()
        .find_map(provider_for)
        .ok_or(MetadataError::NoProvider)?;
    Ok(ClearedSeries {
        series_id,
        name: series.name,
        provider,
        includes_adult: Database::series_adult_metadata_with_connection(conn, series_id)?,
    })
}

/// One entry a provider answered, in the shape every provider's entries take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// The provider's id of the entry, as text: each provider has its own kind of id.
    pub external_id: String,
    pub title: String,
    /// Its other titles — other languages and scripts, and synonyms.
    pub alternative_titles: Vec<String>,
    pub start_year: Option<i64>,
    pub end_year: Option<i64>,
    /// Plain text: no markup, and no spoilers where the provider marks them.
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
}

impl Candidate {
    /// Whether its title or one of its other titles is `name`, as a series' match key normalises
    /// both.
    pub fn is_exact_match(&self, name: &str) -> bool {
        let key = match_key(name.trim());
        std::iter::once(&self.title)
            .chain(&self.alternative_titles)
            .any(|title| match_key(title.trim()) == key)
    }
}

/// Who waits for a provider's turn, which says how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// An admin's request: it waits at most `ADMIN_WAIT`, else answers `RateLimited`.
    Admin,
    /// The bulk pass: it waits as long as the provider asks.
    BulkPass,
}

impl Caller {
    fn max_wait(self) -> Option<Duration> {
        match self {
            Caller::Admin => Some(ADMIN_WAIT),
            Caller::BulkPass => None,
        }
    }
}

/// Every provider, reached through one HTTP client, each at its own pace. Clones share the
/// client and the providers' turns.
#[derive(Debug, Clone)]
pub struct MetadataProviders {
    anilist: Endpoint,
}

impl MetadataProviders {
    /// The providers at their own addresses.
    pub fn new() -> reqwest::Result<Self> {
        let client = http::client(http::TIMEOUT)?;
        Ok(MetadataProviders {
            anilist: Endpoint::new(
                MetadataProvider::AniList,
                anilist::URL,
                client,
                anilist::INTERVAL,
            ),
        })
    }

    /// Every provider at `url` — a test's stand-in — with no pause between requests, each given
    /// up on after `timeout`.
    #[cfg(test)]
    pub fn at(url: &str, timeout: Duration) -> Self {
        let client = http::client(timeout).expect("build the HTTP client");
        MetadataProviders {
            anilist: Endpoint::new(MetadataProvider::AniList, url, client, Duration::ZERO),
        }
    }

    /// The candidates the series' provider finds for `query`, best first: the answer it gave the
    /// same search within 30 days before `now`, else its answer now, which is kept.
    pub async fn search(
        &self,
        db: &Arc<Database>,
        series: &ClearedSeries,
        query: &str,
        caller: Caller,
        now: SystemTime,
    ) -> Result<Vec<Candidate>, AppErrors> {
        // Normalised as a title's match key, so "Berserk" and " berserk" share one answer.
        let key = request_key(series, "search", &match_key(query.trim()));
        let ask = self.search_online(series, query, caller);
        cached(db, series.provider(), key, now, ask).await
    }

    /// The series' provider's entry `external_id`: as it answered within 30 days before `now`,
    /// else as it answers now, which is kept.
    pub async fn fetch(
        &self,
        db: &Arc<Database>,
        series: &ClearedSeries,
        external_id: &str,
        caller: Caller,
        now: SystemTime,
    ) -> Result<Candidate, AppErrors> {
        let key = request_key(series, "entry", external_id);
        let ask = self.fetch_online(series, external_id, caller);
        cached(db, series.provider(), key, now, ask).await
    }

    async fn search_online(
        &self,
        series: &ClearedSeries,
        query: &str,
        caller: Caller,
    ) -> Result<Vec<Candidate>, MetadataError> {
        match series.provider() {
            MetadataProvider::AniList => {
                anilist::search(&self.anilist, series, query, caller).await
            }
        }
    }

    async fn fetch_online(
        &self,
        series: &ClearedSeries,
        external_id: &str,
        caller: Caller,
    ) -> Result<Candidate, MetadataError> {
        match series.provider() {
            MetadataProvider::AniList => {
                anilist::fetch(&self.anilist, series, external_id, caller).await
            }
        }
    }
}

/// The key a lookup's answer is kept under: what was asked — `kind` — and `detail`. A lookup that
/// includes adult entries has keys of its own, as its answers differ.
fn request_key(series: &ClearedSeries, kind: &str, detail: &str) -> String {
    let adult = if series.includes_adult() {
        "+adult"
    } else {
        ""
    };
    format!("{kind}{adult}:{detail}")
}

/// The answer `provider` gave to `key` within 30 days before `now`; else `ask`'s, which is kept.
/// A failure isn't kept, so the next lookup asks again.
async fn cached<T>(
    db: &Arc<Database>,
    provider: MetadataProvider,
    key: String,
    now: SystemTime,
    ask: impl Future<Output = Result<T, MetadataError>>,
) -> Result<T, AppErrors>
where
    T: Serialize + DeserializeOwned,
{
    let now = convert_system_time_to_unix_time(now);
    let read_key = key.clone();
    let kept = Arc::clone(db)
        .read(move |conn| Ok(db_metadata::fresh(conn, provider, &read_key, now)?))
        .await?;
    if let Some(json) = kept {
        match serde_json::from_str(&json) {
            Ok(answer) => return Ok(answer),
            // Kept by an older version, whose answers had another shape.
            Err(err) => tracing::warn!(
                "The kept answer to {key} can't be read, so it is asked again: {err}"
            ),
        }
    }

    let answer = ask.await?;
    match serde_json::to_string(&answer) {
        Ok(json) => {
            Arc::clone(db)
                .transact(move |conn| Ok(db_metadata::keep(conn, provider, &key, &json, now)?))
                .await?
        }
        Err(err) => tracing::error!("The answer to {key} can't be kept: {err}"),
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database_related_scripts::db_metadata::LIFETIME_SECONDS,
        error_types::DatabaseError,
        test_support::{memory_connection, ProviderStub, TempDatabase},
    };
    use axum::http::StatusCode;
    use rusqlite::params;
    use std::time::UNIX_EPOCH;

    fn library(conn: &Connection, library_type: &str, sensitive: bool) -> i64 {
        conn.execute(
            "INSERT INTO library (library_name, library_type, is_sensitive) VALUES (?1, ?1, ?2)",
            params![library_type, sensitive],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn series(conn: &Connection, name: &str, sensitive: bool, libraries: &[i64]) -> i64 {
        conn.execute(
            "INSERT INTO series (name, match_key, is_sensitive) VALUES (?1, lower(?1), ?2)",
            params![name, sensitive],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        for library_id in libraries {
            conn.execute(
                "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                [library_id, &id],
            )
            .unwrap();
        }
        id
    }

    fn refusal(result: Result<ClearedSeries, AppErrors>) -> AppErrors {
        result.expect_err("the series must be refused")
    }

    #[test]
    fn only_manga_has_a_provider_so_far() {
        for library_type in [
            LibraryType::Books,
            LibraryType::Comics,
            LibraryType::Manga,
            LibraryType::Magazines,
            LibraryType::Documents,
            LibraryType::Others,
        ] {
            let expected =
                (library_type == LibraryType::Manga).then_some(MetadataProvider::AniList);
            assert_eq!(provider_for(library_type), expected, "{library_type:?}");
        }
        assert_eq!(MetadataProvider::AniList.as_str(), "anilist");
    }

    #[test]
    fn a_manga_series_is_cleared_for_anilist_under_its_name() {
        let conn = memory_connection();
        let manga = library(&conn, "manga", false);
        let berserk = series(&conn, "Berserk", false, &[manga]);

        let cleared = clear(&conn, berserk).unwrap();

        assert_eq!(cleared.series_id(), berserk);
        assert_eq!(cleared.name(), "Berserk");
        assert_eq!(cleared.provider(), MetadataProvider::AniList);
        assert!(
            !cleared.includes_adult(),
            "off unless a library turns it on"
        );
    }

    #[test]
    fn a_series_includes_adult_entries_when_one_of_its_libraries_has_them_on() {
        let conn = memory_connection();
        let manga = library(&conn, "manga", false);
        let favourites = library(&conn, "others", false);
        let berserk = series(&conn, "Berserk", false, &[manga, favourites]);
        conn.execute(
            "UPDATE library SET adult_metadata = 1 WHERE id = ?1",
            [favourites],
        )
        .unwrap();

        let cleared = clear(&conn, berserk).unwrap();

        assert!(cleared.includes_adult());
        assert_eq!(cleared.provider(), MetadataProvider::AniList);
    }

    #[test]
    fn a_library_of_a_type_without_a_provider_beside_a_manga_one_changes_nothing() {
        let conn = memory_connection();
        let favourites = library(&conn, "others", false);
        let manga = library(&conn, "manga", false);
        let berserk = series(&conn, "Berserk", false, &[favourites, manga]);

        assert_eq!(
            clear(&conn, berserk).unwrap().provider(),
            MetadataProvider::AniList
        );
    }

    #[test]
    fn a_series_marked_sensitive_or_in_a_sensitive_library_is_refused() {
        let conn = memory_connection();
        let manga = library(&conn, "manga", false);
        let private = library(&conn, "manga", true);
        let marked = series(&conn, "Marked", true, &[manga]);
        let shared = series(&conn, "Shared", false, &[manga, private]);
        // Refused as sensitive before its missing provider is even looked for.
        let loose = series(&conn, "Loose", true, &[]);

        for (id, why) in [
            (marked, "its own flag"),
            (shared, "one of its two libraries"),
            (loose, "in no library"),
        ] {
            assert!(
                matches!(
                    refusal(clear(&conn, id)),
                    AppErrors::Metadata(MetadataError::SensitiveItem)
                ),
                "{why}"
            );
        }
    }

    #[test]
    fn a_series_in_no_library_or_only_in_libraries_without_a_provider_has_none() {
        let conn = memory_connection();
        let books = library(&conn, "books", false);
        let comics = library(&conn, "comics", false);
        let loose = series(&conn, "Loose", false, &[]);
        let novel = series(&conn, "Novel", false, &[books, comics]);

        for id in [loose, novel] {
            assert!(matches!(
                refusal(clear(&conn, id)),
                AppErrors::Metadata(MetadataError::NoProvider)
            ));
        }
    }

    #[test]
    fn an_unknown_series_is_not_found() {
        let conn = memory_connection();

        assert!(matches!(
            refusal(clear(&conn, 42)),
            AppErrors::Database(DatabaseError::EntryNotFound)
        ));
    }

    #[test]
    fn a_candidate_matches_a_name_exactly_by_any_of_its_titles_ignoring_case_and_composition() {
        let candidate = Candidate {
            external_id: "53390".to_string(),
            title: "Attack on Titan".to_string(),
            alternative_titles: vec!["Shingeki no Kyojin".to_string(), "Ärger".to_string()],
            start_year: None,
            end_year: None,
            description: None,
            cover_url: None,
            authors: vec![],
            tags: vec![],
        };

        for name in [
            "Attack on Titan",
            " ATTACK ON TITAN ",
            "shingeki no kyojin",
            // "Ä" written as "A" and a combining diaeresis, as older macOS disks store it.
            "A\u{308}rger",
        ] {
            assert!(candidate.is_exact_match(name), "{name:?}");
        }
        for name in ["Attack on Titan 2", "Shingeki", "Attack  on Titan", ""] {
            assert!(!candidate.is_exact_match(name), "{name:?}");
        }
    }

    const SEARCH_ANSWER: &str =
        r#"{"data":{"Page":{"media":[{"id":30002,"title":{"romaji":"Berserk"}}]}}}"#;
    const ENTRY_ANSWER: &str = r#"{"data":{"Media":{"id":30002,"title":{"romaji":"Berserk"}}}}"#;
    const T: u64 = 1_800_000_000;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// A series in a manga library of `temp`, cleared as every lookup's series is.
    fn cleared_in(temp: &TempDatabase, includes_adult: bool) -> ClearedSeries {
        temp.db
            .setup_new_transaction(|conn| {
                let manga = library(conn, "manga", false);
                conn.execute(
                    "UPDATE library SET adult_metadata = ?1 WHERE id = ?2",
                    params![includes_adult, manga],
                )
                .unwrap();
                let berserk = series(conn, "Berserk", false, &[manga]);
                clear(conn, berserk)
            })
            .unwrap()
    }

    /// Every kept answer: its key, its response and when it was fetched.
    fn kept(temp: &TempDatabase) -> Vec<(String, String, i64)> {
        temp.db
            .read_now(|conn| {
                Ok(conn
                    .prepare(
                        "SELECT request_key, response, fetched_at FROM metadata_cache \
                         ORDER BY request_key",
                    )
                    .unwrap()
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_search_asked_again_within_30_days_is_answered_from_the_cache() {
        let temp = TempDatabase::open();
        let stub = ProviderStub::answering(StatusCode::OK, &[], SEARCH_ANSWER).await;
        let providers = MetadataProviders::at(&stub.url, Duration::from_secs(5));
        let berserk = cleared_in(&temp, false);
        let search = |query: &'static str, now| {
            providers.search(&temp.db, &berserk, query, Caller::Admin, now)
        };

        let first = search("Berserk", at(T)).await.unwrap();
        let again = search(" BERSERK ", at(T + LIFETIME_SECONDS as u64 - 1))
            .await
            .unwrap();

        assert_eq!(stub.requests().len(), 1, "the second came from the cache");
        assert_eq!(again, first);
        assert_eq!(first[0].external_id, "30002");
        assert_eq!(kept(&temp)[0].0, "search:berserk");

        search("Berserk", at(T + LIFETIME_SECONDS as u64))
            .await
            .unwrap();

        assert_eq!(stub.requests().len(), 2, "30 days on, it is asked again");
        let kept = kept(&temp);
        assert_eq!(kept.len(), 1, "and replaces the old answer");
        assert_eq!(kept[0].2, (T + LIFETIME_SECONDS as u64) as i64);
    }

    #[tokio::test]
    async fn an_entry_is_kept_by_its_id_and_a_failure_is_not_kept() {
        let temp = TempDatabase::open();
        let found = ProviderStub::answering(StatusCode::OK, &[], ENTRY_ANSWER).await;
        let berserk = cleared_in(&temp, false);
        let providers = MetadataProviders::at(&found.url, Duration::from_secs(5));
        for _ in 0..2 {
            let entry = providers
                .fetch(&temp.db, &berserk, "30002", Caller::Admin, at(T))
                .await
                .unwrap();
            assert_eq!(entry.title, "Berserk");
        }
        assert_eq!(found.requests().len(), 1);

        let missing =
            ProviderStub::answering(StatusCode::NOT_FOUND, &[], r#"{"data":{"Media":null}}"#).await;
        let providers = MetadataProviders::at(&missing.url, Duration::from_secs(5));
        for _ in 0..2 {
            let result = providers
                .fetch(&temp.db, &berserk, "1", Caller::Admin, at(T))
                .await;
            assert!(matches!(
                result,
                Err(AppErrors::Metadata(MetadataError::CandidateNotFound))
            ));
        }
        assert_eq!(missing.requests().len(), 2, "asked again each time");
        let keys: Vec<String> = kept(&temp).into_iter().map(|row| row.0).collect();
        assert_eq!(keys, ["entry:30002"]);
    }

    #[tokio::test]
    async fn a_lookup_that_includes_adult_entries_keeps_answers_of_its_own() {
        let temp = TempDatabase::open();
        let stub = ProviderStub::answering(StatusCode::OK, &[], SEARCH_ANSWER).await;
        let providers = MetadataProviders::at(&stub.url, Duration::from_secs(5));

        for includes_adult in [false, true] {
            let series = cleared_in(&temp, includes_adult);
            providers
                .search(&temp.db, &series, "Berserk", Caller::Admin, at(T))
                .await
                .unwrap();
        }

        assert_eq!(stub.requests().len(), 2);
        let keys: Vec<String> = kept(&temp).into_iter().map(|row| row.0).collect();
        assert_eq!(keys, ["search+adult:berserk", "search:berserk"]);
    }

    #[tokio::test]
    async fn a_kept_answer_that_cannot_be_read_is_asked_again_and_replaced() {
        let temp = TempDatabase::open();
        let stub = ProviderStub::answering(StatusCode::OK, &[], SEARCH_ANSWER).await;
        let providers = MetadataProviders::at(&stub.url, Duration::from_secs(5));
        let berserk = cleared_in(&temp, false);
        temp.db
            .setup_new_transaction(|conn| {
                Ok(db_metadata::keep(
                    conn,
                    MetadataProvider::AniList,
                    "search:berserk",
                    r#"[{"id":"an older shape"}]"#,
                    T as i64,
                )?)
            })
            .unwrap();

        let found = providers
            .search(&temp.db, &berserk, "Berserk", Caller::Admin, at(T))
            .await
            .unwrap();

        assert_eq!(found[0].title, "Berserk");
        assert_eq!(stub.requests().len(), 1);
        let kept = kept(&temp);
        assert_eq!(kept.len(), 1);
        assert!(
            kept[0].1.contains("\"external_id\":\"30002\""),
            "{}",
            kept[0].1
        );
    }
}
