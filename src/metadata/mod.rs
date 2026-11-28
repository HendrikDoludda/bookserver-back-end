//! Metadata from online providers: which provider describes a series, and the gate every lookup
//! passes.

use rusqlite::Connection;

use crate::{
    db::Database,
    error_types::{AppErrors, MetadataError},
    models::{BookSeriesMetadata, LibraryType},
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error_types::DatabaseError, test_support::memory_connection};
    use rusqlite::params;

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
}
