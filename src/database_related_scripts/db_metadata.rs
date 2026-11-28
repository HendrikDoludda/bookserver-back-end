//! The `metadata_cache` table: the providers' answers, kept so a lookup isn't sent twice. Its key
//! is two columns (provider and request), so it gets these dedicated functions instead of the
//! generic `Extract` / `Update` traits.

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    database_related_scripts::{
        db::{logged, Database},
        db_from_row::FromRow,
    },
    error_types::DatabaseError,
    metadata::MetadataProvider,
    models::{CachedAnswer, CachedAnswerColumns, Comparison, QuerySeparator},
};

/// How long an answer is used: entries rarely change, and a month still lets a provider's
/// corrections through.
pub const LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;

/// The answer `provider` gave to `request_key` within the 30 days up to `now`; `None` when it gave
/// none, only an older one, or one stamped after `now`.
pub fn fresh(
    conn: &Connection,
    provider: MetadataProvider,
    request_key: &str,
    now: i64,
) -> Result<Option<String>, DatabaseError> {
    conn.query_row(
        "SELECT provider, request_key, response, fetched_at FROM metadata_cache \
         WHERE provider = ?1 AND request_key = ?2 AND fetched_at > ?3 AND fetched_at <= ?4",
        params![provider.as_str(), request_key, now - LIFETIME_SECONDS, now],
        CachedAnswer::from_row,
    )
    .optional()
    .map(|answer| answer.map(|answer| answer.response))
    .map_err(logged(
        "reading",
        "metadata_cache",
        DatabaseError::QueryFailure,
    ))
}

/// Keeps `response` as `provider`'s answer to `request_key`, fetched at `now`, in place of any
/// earlier one.
pub fn keep(
    conn: &Connection,
    provider: MetadataProvider,
    request_key: &str,
    response: &str,
    now: i64,
) -> Result<(), DatabaseError> {
    conn.execute(
        "INSERT INTO metadata_cache (provider, request_key, response, fetched_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (provider, request_key) DO UPDATE SET \
         response = excluded.response, fetched_at = excluded.fetched_at",
        params![provider.as_str(), request_key, response, now],
    )
    .map(|_| ())
    .map_err(logged(
        "inserting into",
        "metadata_cache",
        DatabaseError::InsertionFailure,
    ))
}

/// Deletes every answer that is no longer fresh at `now`; answers how many.
pub fn delete_expired(conn: &Connection, now: i64) -> Result<usize, DatabaseError> {
    Database::remove_entries_where_with_connection::<CachedAnswer>(
        conn,
        &[(CachedAnswerColumns::FetchedAt, Comparison::AtMost)],
        &[&(now - LIFETIME_SECONDS)],
        QuerySeparator::And,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::memory_connection;

    const NOW: i64 = 1_800_000_000;
    const ANILIST: MetadataProvider = MetadataProvider::AniList;

    fn stored(conn: &Connection) -> Vec<(String, String, String, i64)> {
        conn.prepare(
            "SELECT provider, request_key, response, fetched_at FROM metadata_cache \
             ORDER BY provider, request_key",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn an_answer_is_used_for_30_days_and_then_counts_as_missing() {
        let conn = memory_connection();
        keep(&conn, ANILIST, "search:berserk", "[1]", NOW).unwrap();
        let read = |now| fresh(&conn, ANILIST, "search:berserk", now).unwrap();

        assert_eq!(read(NOW).as_deref(), Some("[1]"));
        assert_eq!(read(NOW + LIFETIME_SECONDS - 1).as_deref(), Some("[1]"));
        assert_eq!(read(NOW + LIFETIME_SECONDS), None, "30 days old");
        assert_eq!(read(NOW - 1), None, "stamped after the time asked about");
    }

    #[test]
    fn a_kept_answer_replaces_the_one_before() {
        let conn = memory_connection();
        keep(&conn, ANILIST, "entry:30002", "old", NOW).unwrap();

        keep(&conn, ANILIST, "entry:30002", "new", NOW + 60).unwrap();

        assert_eq!(
            stored(&conn),
            [(
                "anilist".to_string(),
                "entry:30002".to_string(),
                "new".to_string(),
                NOW + 60
            )]
        );
    }

    #[test]
    fn each_provider_and_request_has_its_own_answer() {
        let conn = memory_connection();
        conn.execute(
            "INSERT INTO metadata_cache (provider, request_key, response, fetched_at) \
             VALUES ('mangadex', 'search:berserk', 'theirs', ?1)",
            [NOW],
        )
        .unwrap();
        keep(&conn, ANILIST, "search:berserk+", "other request", NOW).unwrap();

        assert_eq!(fresh(&conn, ANILIST, "search:berserk", NOW).unwrap(), None);
        keep(&conn, ANILIST, "search:berserk", "ours", NOW).unwrap();
        assert_eq!(
            fresh(&conn, ANILIST, "search:berserk", NOW)
                .unwrap()
                .as_deref(),
            Some("ours")
        );
        assert_eq!(stored(&conn).len(), 3, "the other provider's answer stays");
    }

    #[test]
    fn the_answers_no_longer_fresh_are_deleted_and_counted() {
        let conn = memory_connection();
        for (key, fetched_at) in [
            ("expired long ago", NOW - 2 * LIFETIME_SECONDS),
            ("expired just now", NOW - LIFETIME_SECONDS),
            ("fresh for a second", NOW - LIFETIME_SECONDS + 1),
            ("fresh", NOW),
        ] {
            keep(&conn, ANILIST, key, "[]", fetched_at).unwrap();
        }

        assert_eq!(delete_expired(&conn, NOW).unwrap(), 2);

        let left: Vec<String> = stored(&conn).into_iter().map(|row| row.1).collect();
        assert_eq!(left, ["fresh", "fresh for a second"]);
        for key in left {
            assert!(fresh(&conn, ANILIST, &key, NOW).unwrap().is_some(), "{key}");
        }
    }
}
