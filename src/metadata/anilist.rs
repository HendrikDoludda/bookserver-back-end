//! AniList (anilist.co): manga entries through its GraphQL API, which takes no key.

use std::{cmp::Reverse, time::Duration};

use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};

use super::{http::Endpoint, plain_text::plain_text, Caller, Candidate, ClearedSeries};
use crate::error_types::MetadataError;

pub const URL: &str = "https://graphql.anilist.co";

/// Keeps under the 30 requests a minute AniList allows while degraded, as its
/// `X-RateLimit-Limit` header says.
pub const INTERVAL: Duration = Duration::from_secs(2);

/// Both requests, sent whole with the name of the one to run. The text never changes: what is
/// searched for travels only in `variables`.
const DOCUMENT: &str = concat!(
    "query Search($search: String, $isAdult: Boolean) { Page(perPage: 10) { ",
    "media(search: $search, type: MANGA, isAdult: $isAdult, sort: SEARCH_MATCH) { ...entry } } } ",
    "query Entry($id: Int, $isAdult: Boolean) { ",
    "Media(id: $id, type: MANGA, isAdult: $isAdult) { ...entry } } ",
    "fragment entry on Media { id title { romaji english native } synonyms ",
    "startDate { year } endDate { year } description(asHtml: false) coverImage { extraLarge } ",
    "genres tags { name rank isMediaSpoiler isGeneralSpoiler isAdult } ",
    "staff(perPage: 25, sort: [RELEVANCE, ID]) { edges { role node { name { full } } } } }",
);

/// The staff roles that make someone an author; a note in brackets, as in
/// `Story & Art (vols 1-41)`, doesn't change the role.
const AUTHOR_ROLES: [&str; 4] = ["Story & Art", "Story", "Art", "Original Story"];

/// A tag's rank is how well it fits the entry, 0 to 100 by AniList's votes; below this, a tag is
/// mostly noise.
const LOWEST_TAG_RANK: i64 = 60;

/// A popular entry has dozens of tags above `LOWEST_TAG_RANK`; past the best ten, they crowd a
/// book's tags with noise.
const MOST_TAGS: usize = 10;

/// AniList's ten best matches for `query` among its manga.
pub async fn search(
    endpoint: &Endpoint,
    series: &ClearedSeries,
    query: &str,
    caller: Caller,
) -> Result<Vec<Candidate>, MetadataError> {
    let request = Request {
        query: DOCUMENT,
        operation_name: "Search",
        variables: SearchVariables {
            search: query,
            is_adult: adult_filter(series),
        },
    };
    let (status, body) = endpoint.post_json(&request, caller).await?;
    if status != StatusCode::OK {
        return Err(endpoint.unavailable(format_args!("it answered a search with {status}")));
    }
    let data: SearchData = read(endpoint, &body)?;
    Ok(data
        .page
        .map(|page| page.media)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|media| media.into_candidate(series.includes_adult()))
        .collect())
}

/// AniList's manga entry `external_id`.
pub async fn fetch(
    endpoint: &Endpoint,
    series: &ClearedSeries,
    external_id: &str,
    caller: Caller,
) -> Result<Candidate, MetadataError> {
    let request = Request {
        query: DOCUMENT,
        operation_name: "Entry",
        variables: EntryVariables {
            id: entry_number(external_id)?,
            is_adult: adult_filter(series),
        },
    };
    let (status, body) = endpoint.post_json(&request, caller).await?;
    match status {
        StatusCode::OK => {}
        // How AniList answers an id it has no manga under, or none the filter lets through.
        StatusCode::NOT_FOUND => return Err(MetadataError::CandidateNotFound),
        other => {
            return Err(endpoint.unavailable(format_args!("it answered an entry with {other}")))
        }
    }
    let data: EntryData = read(endpoint, &body)?;
    data.media
        .and_then(|media| media.into_candidate(series.includes_adult()))
        .ok_or(MetadataError::CandidateNotFound)
}

/// An AniList id as it sends one: a positive whole number in plain digits that fits GraphQL's
/// 32-bit `Int`.
fn entry_number(external_id: &str) -> Result<i32, MetadataError> {
    external_id
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| external_id.parse().ok())
        .flatten()
        .filter(|&id| id > 0)
        .ok_or(MetadataError::InvalidExternalId)
}

/// `isAdult` for the series' lookups: `false` keeps adult entries out; left out, both kinds come.
/// Never `null`, which AniList matches against nothing.
fn adult_filter(series: &ClearedSeries) -> Option<bool> {
    (!series.includes_adult()).then_some(false)
}

/// The `data` of an answer; one that isn't JSON of that shape, or has no `data`, is AniList's
/// failure.
fn read<T: DeserializeOwned>(endpoint: &Endpoint, body: &[u8]) -> Result<T, MetadataError> {
    match serde_json::from_slice::<Answer<T>>(body) {
        Ok(Answer { data: Some(data) }) => Ok(data),
        Ok(Answer { data: None }) => Err(endpoint.unavailable("its answer has no data")),
        Err(err) => Err(endpoint.unavailable(format_args!("its answer can't be read: {err}"))),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Request<V> {
    query: &'static str,
    operation_name: &'static str,
    variables: V,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchVariables<'a> {
    search: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_adult: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryVariables {
    id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_adult: Option<bool>,
}

#[derive(Deserialize)]
struct Answer<T> {
    data: Option<T>,
}

#[derive(Deserialize)]
struct SearchData {
    #[serde(rename = "Page")]
    page: Option<Page>,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default, deserialize_with = "items")]
    media: Vec<Media>,
}

#[derive(Deserialize)]
struct EntryData {
    #[serde(rename = "Media")]
    media: Option<Media>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    id: i64,
    title: Option<Title>,
    #[serde(default, deserialize_with = "items")]
    synonyms: Vec<String>,
    start_date: Option<FuzzyDate>,
    end_date: Option<FuzzyDate>,
    description: Option<String>,
    cover_image: Option<CoverImage>,
    #[serde(default, deserialize_with = "items")]
    genres: Vec<String>,
    #[serde(default, deserialize_with = "items")]
    tags: Vec<Tag>,
    staff: Option<StaffConnection>,
}

#[derive(Deserialize, Default)]
struct Title {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
}

#[derive(Deserialize)]
struct FuzzyDate {
    year: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverImage {
    extra_large: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tag {
    name: String,
    rank: Option<i64>,
    is_media_spoiler: Option<bool>,
    is_general_spoiler: Option<bool>,
    is_adult: Option<bool>,
}

#[derive(Deserialize)]
struct StaffConnection {
    #[serde(default, deserialize_with = "items")]
    edges: Vec<StaffEdge>,
}

#[derive(Deserialize)]
struct StaffEdge {
    role: Option<String>,
    node: Option<Staff>,
}

#[derive(Deserialize)]
struct Staff {
    name: Option<StaffName>,
}

#[derive(Deserialize)]
struct StaffName {
    full: Option<String>,
}

/// A GraphQL list, which may be `null` itself and hold `null`s: the items that are there.
fn items<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let list: Option<Vec<Option<T>>> = Option::deserialize(deserializer)?;
    Ok(list.into_iter().flatten().flatten().collect())
}

impl Media {
    /// The entry as a candidate; `None` when it has no title at all.
    fn into_candidate(self, includes_adult: bool) -> Option<Candidate> {
        let title = self.title.unwrap_or_default();
        let mut titles = [title.english, title.romaji, title.native]
            .into_iter()
            .flatten()
            .chain(self.synonyms);
        let main = titles.by_ref().find_map(trimmed)?;
        let mut alternative_titles = Vec::new();
        for other in titles.filter_map(trimmed) {
            if other != main {
                push_new(&mut alternative_titles, other);
            }
        }

        let mut authors = Vec::new();
        for edge in self.staff.map(|staff| staff.edges).unwrap_or_default() {
            let role = edge.role.unwrap_or_default();
            let role = role.split(" (").next().unwrap_or_default().trim();
            if !AUTHOR_ROLES.contains(&role) {
                continue;
            }
            let name = edge
                .node
                .and_then(|node| node.name)
                .and_then(|name| name.full)
                .and_then(trimmed);
            if let Some(name) = name {
                push_new(&mut authors, name);
            }
        }

        let mut fitting: Vec<Tag> = self
            .tags
            .into_iter()
            .filter(|tag| tag.fits(includes_adult))
            .collect();
        // AniList sends its tags best first, but nothing promises it.
        fitting.sort_by_key(|tag| Reverse(tag.rank));
        let best = fitting.into_iter().take(MOST_TAGS).map(|tag| tag.name);
        let mut tags = Vec::new();
        for tag in self.genres.into_iter().chain(best).filter_map(trimmed) {
            push_new(&mut tags, tag);
        }

        Some(Candidate {
            external_id: self.id.to_string(),
            title: main,
            alternative_titles,
            start_year: self.start_date.and_then(|date| date.year),
            end_year: self.end_date.and_then(|date| date.year),
            description: self
                .description
                .and_then(|text| plain_text(&without_spoilers(&text))),
            cover_url: self
                .cover_image
                .and_then(|cover| cover.extra_large)
                .and_then(trimmed),
            authors,
            tags,
        })
    }
}

impl Tag {
    /// Ranked high enough and spoiling nothing; adult only when the lookup includes adult
    /// entries. A flag AniList leaves out counts against the tag.
    fn fits(&self, includes_adult: bool) -> bool {
        self.rank.is_some_and(|rank| rank >= LOWEST_TAG_RANK)
            && self.is_media_spoiler == Some(false)
            && self.is_general_spoiler == Some(false)
            && (includes_adult || self.is_adult == Some(false))
    }
}

/// `text` without AniList's spoiler spans, `~!` to `!~`; one never closed runs to the end.
fn without_spoilers(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("~!") {
        kept.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .split_once("!~")
            .map_or("", |(_, after)| after);
    }
    kept.push_str(rest);
    kept
}

fn trimmed(text: String) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn push_new(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use axum::http::StatusCode as Status;
    use serde_json::{json, Value};

    use super::{super::http::MAX_ANSWER_BYTES, *};
    use crate::{
        metadata::{clear, MetadataProviders},
        test_support::{memory_connection, unused_port, ProviderStub},
    };

    /// Shaped like AniList's answer to the search for Berserk, trimmed to two entries; the second
    /// entry's adult tag and the spoiler span are added.
    const SEARCH_ANSWER: &str = r#"{"data":{"Page":{"media":[
      {"id":30002,
       "title":{"romaji":"Berserk","english":"Berserk","native":"ベルセルク"},
       "synonyms":["Beruseruku","Berserk Max",null],
       "startDate":{"year":1989},"endDate":{"year":null},
       "description":"His name is Guts, the Black Swordsman.<br>\n<br>\n(Source: Dark Horse)",
       "coverImage":{"extraLarge":"https://s4.anilist.co/file/anilistcdn/media/manga/cover/large/bx30002-Cul4OeN7bYtn.jpg"},
       "genres":["Action","Drama"],
       "tags":[
         {"name":"Tragedy","rank":97,"isMediaSpoiler":false,"isGeneralSpoiler":true,"isAdult":false},
         {"name":"Seinen","rank":95,"isMediaSpoiler":false,"isGeneralSpoiler":false,"isAdult":false},
         {"name":"Cosmic Horror","rank":90,"isMediaSpoiler":true,"isGeneralSpoiler":false,"isAdult":false},
         {"name":"Female Harem","rank":42,"isMediaSpoiler":false,"isGeneralSpoiler":false,"isAdult":false}],
       "staff":{"edges":[
         {"role":"Story & Art (vols 1-41)","node":{"name":{"full":"Kentarou Miura"}}},
         {"role":"Supervisor (vols 41- )","node":{"name":{"full":"Kouji Mori"}}},
         {"role":"Story & Art (vols 41- )","node":{"name":{"full":"Studio Gaga"}}},
         {"role":"Translator (English: vols 1-4, 9)","node":{"name":{"full":"Jason DeAngelis"}}}]}},
      {"id":36408,
       "title":{"romaji":"BERSERK THE PROTOTYPE","english":null,"native":"BERSERK THE PROTOTYPE"},
       "synonyms":null,
       "startDate":{"year":1988},"endDate":{"year":1988},
       "description":"The first version of Guts. ~!He loses.!~ Drawn in 1988.",
       "coverImage":{"extraLarge":null},
       "genres":["Action","Horror"],
       "tags":[
         {"name":"Nudity","rank":70,"isMediaSpoiler":false,"isGeneralSpoiler":false,"isAdult":true}],
       "staff":{"edges":[{"role":"Story & Art","node":{"name":{"full":"Kentarou Miura"}}}]}}
    ]}}}"#;

    const NOT_FOUND_ANSWER: &str = r#"{"errors":[{"message":"Not Found.","status":404,"locations":[{"line":1,"column":206}]}],"data":{"Media":null}}"#;

    /// A series in a manga library, cleared as every lookup's series is.
    fn cleared(includes_adult: bool) -> ClearedSeries {
        let conn = memory_connection();
        conn.execute(
            "INSERT INTO library (library_name, library_type, adult_metadata) \
             VALUES ('Manga', 'manga', ?1)",
            [includes_adult],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series (name, match_key) VALUES ('Berserk', 'berserk')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO library_elements (library_id, series_id) VALUES (1, 1)",
            [],
        )
        .unwrap();
        clear(&conn, 1).unwrap()
    }

    fn providers_at(stub: &ProviderStub) -> MetadataProviders {
        MetadataProviders::at(&stub.url, Duration::from_secs(5))
    }

    fn entry_answer(entry: Value) -> String {
        json!({ "data": { "Media": entry } }).to_string()
    }

    fn the_first_entry() -> Value {
        let answer: Value = serde_json::from_str(SEARCH_ANSWER).unwrap();
        answer["data"]["Page"]["media"][0].clone()
    }

    #[tokio::test]
    async fn a_search_sends_the_constant_document_and_the_query_only_as_a_variable() {
        let stub = ProviderStub::answering(Status::OK, &[], SEARCH_ANSWER).await;
        let query = r#"Berserk" } } query { Viewer { id } } #"#;

        providers_at(&stub)
            .search(&cleared(false), query, Caller::Admin)
            .await
            .unwrap();

        assert_eq!(
            stub.requests(),
            [json!({
                "query": DOCUMENT,
                "operationName": "Search",
                "variables": { "search": query, "isAdult": false },
            })]
        );
    }

    #[tokio::test]
    async fn a_search_reads_each_entry_as_a_candidate() {
        let stub = ProviderStub::answering(Status::OK, &[], SEARCH_ANSWER).await;

        let candidates = providers_at(&stub)
            .search(&cleared(false), "Berserk", Caller::Admin)
            .await
            .unwrap();

        assert_eq!(
            candidates,
            [
                Candidate {
                    external_id: "30002".to_string(),
                    title: "Berserk".to_string(),
                    alternative_titles: vec![
                        "ベルセルク".to_string(),
                        "Beruseruku".to_string(),
                        "Berserk Max".to_string(),
                    ],
                    start_year: Some(1989),
                    end_year: None,
                    description: Some(
                        "His name is Guts, the Black Swordsman.\n\n(Source: Dark Horse)"
                            .to_string()
                    ),
                    cover_url: Some(
                        "https://s4.anilist.co/file/anilistcdn/media/manga/cover/large/bx30002-Cul4OeN7bYtn.jpg"
                            .to_string()
                    ),
                    authors: vec!["Kentarou Miura".to_string(), "Studio Gaga".to_string()],
                    tags: vec![
                        "Action".to_string(),
                        "Drama".to_string(),
                        "Seinen".to_string()
                    ],
                },
                Candidate {
                    external_id: "36408".to_string(),
                    title: "BERSERK THE PROTOTYPE".to_string(),
                    alternative_titles: vec![],
                    start_year: Some(1988),
                    end_year: Some(1988),
                    description: Some("The first version of Guts.  Drawn in 1988.".to_string()),
                    cover_url: None,
                    authors: vec!["Kentarou Miura".to_string()],
                    tags: vec!["Action".to_string(), "Horror".to_string()],
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_series_whose_library_searches_adult_entries_sends_no_filter_and_keeps_adult_tags() {
        let stub = ProviderStub::answering(Status::OK, &[], SEARCH_ANSWER).await;

        let candidates = providers_at(&stub)
            .search(&cleared(true), "Berserk", Caller::Admin)
            .await
            .unwrap();

        assert_eq!(
            stub.requests()[0]["variables"],
            json!({ "search": "Berserk" }),
            "no isAdult at all"
        );
        assert_eq!(candidates[1].tags, ["Action", "Horror", "Nudity"]);
    }

    #[tokio::test]
    async fn a_fetch_asks_for_the_entry_by_its_number_and_reads_it() {
        let stub = ProviderStub::answering(Status::OK, &[], &entry_answer(the_first_entry())).await;
        let providers = providers_at(&stub);

        let candidate = providers
            .fetch(&cleared(false), "30002", Caller::Admin)
            .await
            .unwrap();
        providers
            .fetch(&cleared(true), "0030002", Caller::BulkPass)
            .await
            .unwrap();

        assert_eq!(candidate.external_id, "30002");
        assert_eq!(candidate.title, "Berserk");
        assert_eq!(candidate.authors, ["Kentarou Miura", "Studio Gaga"]);
        let requests = stub.requests();
        assert_eq!(
            requests[0],
            json!({
                "query": DOCUMENT,
                "operationName": "Entry",
                "variables": { "id": 30002, "isAdult": false },
            })
        );
        assert_eq!(requests[1]["variables"], json!({ "id": 30002 }));
    }

    #[tokio::test]
    async fn an_entry_anilist_has_not_or_that_has_no_title_is_not_found() {
        let untitled = {
            let mut entry = the_first_entry();
            entry["title"] = json!({ "romaji": " ", "english": null, "native": null });
            entry["synonyms"] = json!([]);
            entry
        };
        for (status, answer) in [
            (Status::NOT_FOUND, NOT_FOUND_ANSWER.to_string()),
            (Status::OK, entry_answer(Value::Null)),
            (Status::OK, entry_answer(untitled)),
        ] {
            let stub = ProviderStub::answering(status, &[], &answer).await;

            let result = providers_at(&stub)
                .fetch(&cleared(false), "116671", Caller::Admin)
                .await;

            assert!(
                matches!(result, Err(MetadataError::CandidateNotFound)),
                "{status} {answer}"
            );
        }
    }

    #[tokio::test]
    async fn an_id_that_is_no_positive_32_bit_number_is_refused_before_any_request() {
        let stub = ProviderStub::answering(Status::OK, &[], SEARCH_ANSWER).await;
        let providers = providers_at(&stub);

        for refused in [
            "",
            "abc",
            "+5",
            "-5",
            "0",
            "000",
            "12a",
            " 5",
            "5 ",
            "1.5",
            "٣",
            "2147483648",
        ] {
            let result = providers
                .fetch(&cleared(false), refused, Caller::Admin)
                .await;
            assert!(
                matches!(result, Err(MetadataError::InvalidExternalId)),
                "{refused:?}"
            );
        }
        assert!(stub.requests().is_empty());
    }

    #[tokio::test]
    async fn a_429_answers_rate_limited_and_holds_back_the_next_request() {
        let stub = ProviderStub::answering(
            Status::TOO_MANY_REQUESTS,
            &[("retry-after", "30")],
            r#"{"data":null,"errors":[{"message":"Too Many Requests.","status":429}]}"#,
        )
        .await;
        let providers = providers_at(&stub);
        let series = cleared(false);

        let first = providers.search(&series, "Berserk", Caller::Admin).await;
        let second = providers.fetch(&series, "30002", Caller::Admin).await;

        assert!(matches!(
            first,
            Err(MetadataError::RateLimited { retry_after }) if retry_after == Duration::from_secs(30)
        ));
        assert!(
            matches!(
                second,
                Err(MetadataError::RateLimited { retry_after })
                    if retry_after > Duration::from_secs(29) && retry_after <= Duration::from_secs(30)
            ),
            "{second:?}"
        );
        assert_eq!(stub.requests().len(), 1, "the second never went out");
    }

    #[tokio::test]
    async fn an_answer_that_cannot_be_used_makes_the_provider_unavailable() {
        // A good answer, but longer than any is read.
        let too_long = format!("{SEARCH_ANSWER}{}", " ".repeat(MAX_ANSWER_BYTES));
        for (status, answer) in [
            (Status::INTERNAL_SERVER_ERROR, "{}"),
            (Status::SERVICE_UNAVAILABLE, "{}"),
            (
                Status::BAD_REQUEST,
                r#"{"errors":[{"message":"Syntax Error"}]}"#,
            ),
            (Status::NOT_FOUND, NOT_FOUND_ANSWER),
            (Status::OK, "<html>Maintenance</html>"),
            (Status::OK, r#"{"data":null}"#),
            (Status::OK, r#"{"data":{"Page":{"media":[{"title":{}}]}}}"#),
            (Status::OK, too_long.as_str()),
        ] {
            let stub = ProviderStub::answering(status, &[], answer).await;

            let result = providers_at(&stub)
                .search(&cleared(false), "Berserk", Caller::Admin)
                .await;

            assert!(
                matches!(result, Err(MetadataError::ProviderUnavailable)),
                "{status} {}",
                &answer[..answer.len().min(60)]
            );
        }
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        let elsewhere = ProviderStub::answering(Status::OK, &[], SEARCH_ANSWER).await;
        let stub = ProviderStub::answering(
            Status::MOVED_PERMANENTLY,
            &[("location", elsewhere.url.as_str())],
            "",
        )
        .await;

        let result = providers_at(&stub)
            .search(&cleared(false), "Berserk", Caller::Admin)
            .await;

        assert!(matches!(result, Err(MetadataError::ProviderUnavailable)));
        assert!(elsewhere.requests().is_empty());
    }

    #[tokio::test]
    async fn a_provider_that_cannot_be_reached_or_never_answers_is_unavailable() {
        let nowhere = MetadataProviders::at(
            &format!("http://127.0.0.1:{}", unused_port()),
            Duration::from_secs(5),
        );
        let result = nowhere
            .search(&cleared(false), "Berserk", Caller::Admin)
            .await;
        assert!(matches!(result, Err(MetadataError::ProviderUnavailable)));

        let silent = ProviderStub::silent().await;
        let start = Instant::now();
        let result = MetadataProviders::at(&silent.url, Duration::from_millis(300))
            .search(&cleared(false), "Berserk", Caller::Admin)
            .await;
        assert!(matches!(result, Err(MetadataError::ProviderUnavailable)));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "gave up at its timeout"
        );
        assert_eq!(silent.requests().len(), 1, "it was asked");
    }

    #[test]
    fn the_genres_come_first_then_the_ten_best_fitting_tags() {
        let ranks = [61, 99, 70, 75, 80, 85, 90, 95, 65, 60, 98, 97];
        let tags: Vec<Value> = ranks
            .iter()
            .map(|rank| {
                json!({ "name": format!("Rank {rank}"), "rank": rank,
                        "isMediaSpoiler": false, "isGeneralSpoiler": false, "isAdult": false })
            })
            .collect();
        let media: Media = serde_json::from_value(json!({
            "id": 1,
            "title": { "romaji": "Many Tags" },
            "genres": ["Drama", "Rank 99"],
            "tags": tags,
        }))
        .unwrap();

        let candidate = media.into_candidate(false).unwrap();

        assert_eq!(
            candidate.tags,
            [
                "Drama", "Rank 99", "Rank 98", "Rank 97", "Rank 95", "Rank 90", "Rank 85",
                "Rank 80", "Rank 75", "Rank 70", "Rank 65"
            ],
            "Rank 99 is a genre already; 61 and 60 are past the best ten"
        );
    }

    #[test]
    fn spoiler_spans_are_left_out_even_one_never_closed() {
        assert_eq!(without_spoilers("A ~!B!~ C ~!D!~"), "A  C ");
        assert_eq!(without_spoilers("A ~!B and the rest"), "A ");
        assert_eq!(without_spoilers("No spoiler! ~ here"), "No spoiler! ~ here");
    }
}
