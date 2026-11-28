//! What an admin changes about libraries and series: creating and editing libraries, which series
//! they hold, editing and deleting series, and the covers both show.

use std::{collections::BTreeSet, ops::RangeInclusive, sync::Arc};

use rusqlite::{Connection, ToSql};

use crate::{
    data_models::books_model::{
        LibraryChanges, LibrarySummary, NewLibrary, SeriesChanges, SeriesListing, SeriesSummary,
    },
    database_related_scripts::db_reading_modes,
    db::Database,
    error_types::{AppErrors, DatabaseError, LibraryError},
    models::{
        BookDatabaseColumns, BookMetadata, BookSeriesMetadata, DatabaseTypes,
        LibraryDatabaseColumns, LibraryMetadata, SeriesDatabaseColumns, SeriesLibraryConnection,
        SeriesMatchKey, WithId,
    },
    routes::covers,
};

/// Room for any real title, while a pasted page of text is still refused.
const MAX_NAME_CHARS: usize = 200;

/// Four-digit years: no release lies outside them, and a typo such as 20255 is caught.
const RELEASE_YEARS: RangeInclusive<i64> = 1..=9999;

/// A library's or series' name as it is stored: trimmed, and 1 to 200 characters long.
pub fn checked_name(name: &str) -> Result<String, LibraryError> {
    let name = name.trim();
    if (1..=MAX_NAME_CHARS).contains(&name.chars().count()) {
        Ok(name.to_string())
    } else {
        Err(LibraryError::InvalidName)
    }
}

/// Creates a library, holding no series yet, and answers it.
pub async fn create_library(
    db: Arc<Database>,
    new: NewLibrary,
) -> Result<WithId<LibrarySummary>, AppErrors> {
    let library = LibraryMetadata {
        name: checked_name(&new.name)?,
        library_type: new.library_type,
        cover_image: None,
        description: new.description,
        is_sensitive: false,
        adult_metadata: false,
    };
    db.transact(move |conn| {
        let id = Database::insert_with_connection(conn, &library)?;
        Ok(WithId {
            id,
            data: LibrarySummary::new(library, 0),
        })
    })
    .await
}

/// Applies an admin's changes to a library and answers it as it is now. A body that changes
/// nothing is refused.
pub async fn change_library(
    db: Arc<Database>,
    library_id: i64,
    changes: LibraryChanges,
) -> Result<WithId<LibrarySummary>, AppErrors> {
    let LibraryChanges {
        name,
        library_type,
        description,
        adult_metadata,
    } = changes;
    if name.is_none() && library_type.is_none() && description.is_none() && adult_metadata.is_none()
    {
        return Err(AppErrors::InvalidRequest);
    }
    let name = name.as_deref().map(checked_name).transpose()?;
    let library_type = library_type.map(|library_type| library_type.as_str());

    db.transact(move |conn| {
        // Read first: an unknown id answers 404 instead of quietly changing nothing.
        Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
        let mut columns = Vec::new();
        let mut values: Vec<&dyn ToSql> = Vec::new();
        if let Some(name) = &name {
            columns.push(LibraryDatabaseColumns::Name);
            values.push(name);
        }
        if let Some(library_type) = &library_type {
            columns.push(LibraryDatabaseColumns::LibraryType);
            values.push(library_type);
        }
        if let Some(description) = &description {
            columns.push(LibraryDatabaseColumns::Description);
            values.push(description);
        }
        if let Some(adult_metadata) = &adult_metadata {
            columns.push(LibraryDatabaseColumns::AdultMetadata);
            values.push(adult_metadata);
        }
        Database::update_multiple_values_with_connection::<LibraryMetadata>(
            conn, &columns, &values, library_id,
        )?;
        library_summary(conn, library_id)
    })
    .await
}

/// Every series by name, with its number of books and the libraries it is in.
pub async fn all_series(db: Arc<Database>) -> Result<Vec<WithId<SeriesListing>>, AppErrors> {
    db.read(|conn| {
        let ids = Database::all_series_ids_by_name_with_connection(conn)?;
        let series = Database::get_entries_batch_with_connection::<BookSeriesMetadata>(conn, &ids)?;
        let book_counts = Database::count_books_per_series_with_connection(conn)?;
        let mut libraries = Database::libraries_per_series_with_connection(conn)?;
        Ok(series
            .into_iter()
            .map(|one| WithId {
                id: one.id,
                data: SeriesListing {
                    series: SeriesSummary::new(
                        one.data,
                        book_counts.get(&one.id).copied().unwrap_or(0),
                    ),
                    library_ids: libraries.remove(&one.id).unwrap_or_default(),
                },
            })
            .collect())
    })
    .await
}

/// Puts a series into a library; one that is already there stays as it is.
pub async fn add_series(
    db: Arc<Database>,
    library_id: i64,
    series_id: i64,
) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        existing(conn, library_id, series_id)?;
        // Inside the IMMEDIATE transaction no other writer can add the same pair between the
        // read and the insert.
        if !Database::library_holds_series_with_connection(conn, library_id, series_id)? {
            Database::insert_with_connection(
                conn,
                &SeriesLibraryConnection {
                    series_id,
                    library_id,
                },
            )?;
        }
        Ok(())
    })
    .await
}

/// Takes a series out of a library; the series and its books stay.
pub async fn remove_series(
    db: Arc<Database>,
    library_id: i64,
    series_id: i64,
) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        existing(conn, library_id, series_id)?;
        Database::remove_entry_from_library_elements_with_connection(conn, series_id, library_id)?;
        Ok(())
    })
    .await
}

/// Applies an admin's changes to a series and answers it as a library's series list sends it. A
/// body that changes nothing is refused.
pub async fn change_series(
    db: Arc<Database>,
    series_id: i64,
    changes: SeriesChanges,
) -> Result<WithId<SeriesSummary>, AppErrors> {
    let SeriesChanges {
        name,
        description,
        start_release_year,
        end_release_year,
    } = changes;
    if name.is_none()
        && description.is_none()
        && start_release_year.is_none()
        && end_release_year.is_none()
    {
        return Err(AppErrors::InvalidRequest);
    }
    let name = name.as_deref().map(checked_name).transpose()?;
    let mut years_sent = [start_release_year, end_release_year]
        .into_iter()
        .flatten()
        .flatten();
    if years_sent.any(|year| !RELEASE_YEARS.contains(&year)) {
        return Err(LibraryError::InvalidReleaseYears.into());
    }

    db.transact(move |conn| {
        let stored = Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
        // A year sent alone is compared with the other one as it is stored.
        let start = start_release_year.unwrap_or(stored.start_release_year);
        let end = end_release_year.unwrap_or(stored.end_release_year);
        if matches!((start, end), (Some(start), Some(end)) if start > end) {
            return Err(LibraryError::InvalidReleaseYears.into());
        }
        let mut columns = Vec::new();
        let mut values: Vec<&dyn ToSql> = Vec::new();
        if let Some(name) = &name {
            columns.push(SeriesDatabaseColumns::Name);
            values.push(name);
        }
        if let Some(description) = &description {
            columns.push(SeriesDatabaseColumns::Description);
            values.push(description);
        }
        if let Some(year) = &start_release_year {
            columns.push(SeriesDatabaseColumns::StartReleaseYear);
            values.push(year);
        }
        if let Some(year) = &end_release_year {
            columns.push(SeriesDatabaseColumns::EndReleaseYear);
            values.push(year);
        }
        Database::update_multiple_values_with_connection::<BookSeriesMetadata>(
            conn, &columns, &values, series_id,
        )?;
        series_summary(conn, series_id)
    })
    .await
}

/// Deletes a series that holds no books, together with its library links and reading modes; a
/// series with books is refused.
pub async fn delete_series(db: Arc<Database>, series_id: i64) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
        if !Database::get_books_in_series_with_connection(conn, series_id)?.is_empty() {
            return Err(LibraryError::SeriesNotEmpty.into());
        }
        Database::remove_entry_with_connection(conn, DatabaseTypes::Series, series_id)?;
        Ok(())
    })
    .await
}

/// Merges series `series_id` into series `into_id` and answers the kept series as a library's
/// series list sends it. The merged series' books, libraries, reading modes and keys move over,
/// its fields fill only what the kept series leaves empty, and it is deleted. Merging a series
/// into itself is refused.
pub async fn merge_series(
    db: Arc<Database>,
    into_id: i64,
    series_id: i64,
) -> Result<WithId<SeriesSummary>, AppErrors> {
    if series_id == into_id {
        return Err(AppErrors::InvalidRequest);
    }
    db.transact(move |conn| {
        let merged = Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
        let kept = Database::get_entry_with_connection::<BookSeriesMetadata>(conn, into_id)?;

        // Before the delete below, which `books.series` (`ON DELETE RESTRICT`) would refuse.
        for book_id in Database::get_books_in_series_with_connection(conn, series_id)? {
            Database::update_value_with_connection::<BookMetadata>(
                conn,
                BookDatabaseColumns::Series,
                &into_id,
                book_id,
            )?;
        }
        let libraries = Database::libraries_per_series_with_connection(conn)?
            .remove(&series_id)
            .unwrap_or_default();
        for library_id in libraries {
            if !Database::library_holds_series_with_connection(conn, library_id, into_id)? {
                Database::insert_with_connection(
                    conn,
                    &SeriesLibraryConnection {
                        series_id: into_id,
                        library_id,
                    },
                )?;
            }
        }
        db_reading_modes::carry_over(conn, series_id, into_id)?;
        fill_empty_fields(conn, into_id, &kept, &merged)?;
        take_over_keys(conn, series_id, into_id, &kept, &merged)?;
        Database::remove_entry_with_connection(conn, DatabaseTypes::Series, series_id)?;
        series_summary(conn, into_id)
    })
    .await
}

/// Gives the kept series the merged one's description, cover and release years where it has none.
fn fill_empty_fields(
    conn: &Connection,
    into_id: i64,
    kept: &BookSeriesMetadata,
    merged: &BookSeriesMetadata,
) -> Result<(), AppErrors> {
    let mut columns = Vec::new();
    let mut values: Vec<&dyn ToSql> = Vec::new();
    if kept.description.is_none() && merged.description.is_some() {
        columns.push(SeriesDatabaseColumns::Description);
        values.push(&merged.description);
    }
    if kept.cover_image.is_none() && merged.cover_image.is_some() {
        columns.push(SeriesDatabaseColumns::CoverImage);
        values.push(&merged.cover_image);
    }
    // As a pair: the kept series' first year with the merged one's last could be out of order.
    let kept_has_years = kept.start_release_year.is_some() || kept.end_release_year.is_some();
    let merged_has_years = merged.start_release_year.is_some() || merged.end_release_year.is_some();
    if !kept_has_years && merged_has_years {
        columns.push(SeriesDatabaseColumns::StartReleaseYear);
        values.push(&merged.start_release_year);
        columns.push(SeriesDatabaseColumns::EndReleaseYear);
        values.push(&merged.end_release_year);
    }
    if !columns.is_empty() {
        Database::update_multiple_values_with_connection::<BookSeriesMetadata>(
            conn, &columns, &values, into_id,
        )?;
    }
    Ok(())
}

/// Gives the kept series every key the merged one answers to that it doesn't answer to already.
fn take_over_keys(
    conn: &Connection,
    series_id: i64,
    into_id: i64,
    kept: &BookSeriesMetadata,
    merged: &BookSeriesMetadata,
) -> Result<(), AppErrors> {
    let mut known: BTreeSet<String> = Database::extra_match_keys_with_connection(conn, into_id)?
        .into_iter()
        .collect();
    known.insert(kept.match_key.clone());
    let mut keys = Database::extra_match_keys_with_connection(conn, series_id)?;
    keys.push(merged.match_key.clone());
    for match_key in keys {
        if known.insert(match_key.clone()) {
            Database::insert_with_connection(
                conn,
                &SeriesMatchKey {
                    series_id: into_id,
                    match_key,
                },
            )?;
        }
    }
    Ok(())
}

/// The series shows the cover of one of its books. A book of another series, or one without a
/// cover, is refused.
pub async fn choose_series_cover(
    db: Arc<Database>,
    series_id: i64,
    book_id: i64,
) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
        let cover = match Database::get_entry_with_connection::<BookMetadata>(conn, book_id) {
            Ok(book) if book.series == Some(series_id) => book.cover_image,
            Ok(_) | Err(DatabaseError::EntryNotFound) => None,
            Err(error) => return Err(error.into()),
        };
        let cover = cover.ok_or(AppErrors::InvalidRequest)?;
        store_series_cover(conn, series_id, Some(cover))
    })
    .await
}

/// The series shows its automatic cover again.
pub async fn reset_series_cover(db: Arc<Database>, series_id: i64) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
        store_series_cover(conn, series_id, None)
    })
    .await
}

/// The library shows the cover one of its series shows now — its own, or the one it falls back
/// to — and keeps that picture when the series' cover changes later. A series the library
/// doesn't hold, or one without a cover, is refused.
pub async fn choose_library_cover(
    db: Arc<Database>,
    library_id: i64,
    series_id: i64,
) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
        if !Database::library_holds_series_with_connection(conn, library_id, series_id)? {
            return Err(AppErrors::InvalidRequest);
        }
        let cover = covers::series_cover(conn, series_id)?.ok_or(AppErrors::InvalidRequest)?;
        store_library_cover(conn, library_id, Some(cover))
    })
    .await
}

/// The library shows its automatic cover again.
pub async fn reset_library_cover(db: Arc<Database>, library_id: i64) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
        store_library_cover(conn, library_id, None)
    })
    .await
}

/// Stores the cover file a series shows; `None` leaves it to the automatic one.
fn store_series_cover(
    conn: &Connection,
    series_id: i64,
    cover: Option<String>,
) -> Result<(), AppErrors> {
    Database::update_multiple_values_with_connection::<BookSeriesMetadata>(
        conn,
        &[SeriesDatabaseColumns::CoverImage],
        &[&cover],
        series_id,
    )?;
    Ok(())
}

/// Stores the cover file a library shows; `None` leaves it to the automatic one.
fn store_library_cover(
    conn: &Connection,
    library_id: i64,
    cover: Option<String>,
) -> Result<(), AppErrors> {
    Database::update_multiple_values_with_connection::<LibraryMetadata>(
        conn,
        &[LibraryDatabaseColumns::CoverImage],
        &[&cover],
        library_id,
    )?;
    Ok(())
}

/// `DatabaseError::EntryNotFound` unless both the library and the series exist.
fn existing(conn: &Connection, library_id: i64, series_id: i64) -> Result<(), DatabaseError> {
    Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
    Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
    Ok(())
}

/// The series as a library's series list sends it.
fn series_summary(conn: &Connection, series_id: i64) -> Result<WithId<SeriesSummary>, AppErrors> {
    let series = Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
    let book_count = Database::get_books_in_series_with_connection(conn, series_id)?.len() as i64;
    Ok(WithId {
        id: series_id,
        data: SeriesSummary::new(series, book_count),
    })
}

/// The library as the library list sends it.
fn library_summary(
    conn: &Connection,
    library_id: i64,
) -> Result<WithId<LibrarySummary>, AppErrors> {
    let library = Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
    let series_count =
        Database::get_series_entries_in_library_with_connection(conn, library_id)?.len() as i64;
    Ok(WithId {
        id: library_id,
        data: LibrarySummary::new(library, series_count),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::LibraryType, test_support::TempDatabase};

    fn new_library(name: &str) -> NewLibrary {
        NewLibrary {
            name: name.to_string(),
            library_type: LibraryType::Comics,
            description: Some("Picked by hand".to_string()),
        }
    }

    fn changes(
        name: Option<&str>,
        library_type: Option<LibraryType>,
        description: Option<Option<&str>>,
    ) -> LibraryChanges {
        LibraryChanges {
            name: name.map(String::from),
            library_type,
            description: description.map(|text| text.map(String::from)),
            adult_metadata: None,
        }
    }

    fn add_series_row(temp: &TempDatabase, name: &str) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
                    .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }

    fn linked_libraries(temp: &TempDatabase, series_id: i64) -> Vec<i64> {
        temp.db
            .setup_new_transaction(|conn| {
                Ok(Database::libraries_per_series_with_connection(conn)?
                    .remove(&series_id)
                    .unwrap_or_default())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_new_library_has_its_name_trimmed_and_holds_no_series() {
        let temp = TempDatabase::open();

        let created = create_library(temp.db.clone(), new_library("  Favourites \n"))
            .await
            .unwrap();

        assert_eq!(created.data.name, "Favourites");
        assert_eq!(created.data.library_type, LibraryType::Comics);
        assert_eq!(created.data.description.as_deref(), Some("Picked by hand"));
        assert_eq!(created.data.series_count, 0);
        let stored = temp.db.get_entry::<LibraryMetadata>(created.id).unwrap();
        assert_eq!(stored.name, "Favourites");
        assert_eq!(stored.library_type.as_str(), "comics");
    }

    #[tokio::test]
    async fn a_name_must_have_1_to_200_characters_after_trimming() {
        let temp = TempDatabase::open();
        for refused in ["", "   ", &"x".repeat(201)] {
            let result = create_library(temp.db.clone(), new_library(refused)).await;
            assert!(
                matches!(result, Err(AppErrors::Library(LibraryError::InvalidName))),
                "{refused:?}"
            );
        }
        // Characters, not bytes: 200 umlauts are 400 bytes.
        for accepted in ["x".repeat(200), "ü".repeat(200)] {
            assert!(create_library(temp.db.clone(), new_library(&accepted))
                .await
                .is_ok());
        }
        let stored: i64 = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM library", [], |row| row.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(stored, 2, "a refused name stores nothing");
    }

    #[tokio::test]
    async fn a_change_touches_only_the_fields_it_names_and_null_clears_the_description() {
        let temp = TempDatabase::open();
        let id = create_library(temp.db.clone(), new_library("Manga"))
            .await
            .unwrap()
            .id;
        let change = |name, library_type, description| {
            change_library(
                temp.db.clone(),
                id,
                changes(name, library_type, description),
            )
        };

        let renamed = change(Some(" Manga & more "), None, None).await.unwrap();
        assert_eq!(renamed.data.name, "Manga & more");
        assert_eq!(renamed.data.library_type, LibraryType::Comics);
        assert_eq!(renamed.data.description.as_deref(), Some("Picked by hand"));

        let retyped = change(None, Some(LibraryType::Books), None).await.unwrap();
        assert_eq!(retyped.data.library_type, LibraryType::Books);
        assert_eq!(retyped.data.name, "Manga & more");

        let described = change(None, None, Some(Some("Japanese comics")))
            .await
            .unwrap();
        assert_eq!(
            described.data.description.as_deref(),
            Some("Japanese comics")
        );

        let cleared = change(None, None, Some(None)).await.unwrap();
        assert_eq!(cleared.data.description, None);
        assert_eq!(cleared.data.name, "Manga & more");
    }

    #[tokio::test]
    async fn a_new_library_searches_no_adult_entries_until_a_change_turns_it_on() {
        let temp = TempDatabase::open();
        let created = create_library(temp.db.clone(), new_library("Manga"))
            .await
            .unwrap();
        assert!(!created.data.adult_metadata);
        let toggle = |on| {
            change_library(
                temp.db.clone(),
                created.id,
                LibraryChanges {
                    adult_metadata: Some(on),
                    ..changes(None, None, None)
                },
            )
        };

        let on = toggle(true).await.unwrap();
        assert!(on.data.adult_metadata);
        assert_eq!(on.data.name, "Manga", "nothing else changes");
        assert_eq!(on.data.description.as_deref(), Some("Picked by hand"));
        let stored = temp.db.get_entry::<LibraryMetadata>(created.id).unwrap();
        assert!(stored.adult_metadata);
        assert!(!stored.is_sensitive, "a different switch");

        let off = toggle(false).await.unwrap();
        assert!(!off.data.adult_metadata);
        assert!(
            !temp
                .db
                .get_entry::<LibraryMetadata>(created.id)
                .unwrap()
                .adult_metadata
        );
    }

    #[tokio::test]
    async fn a_library_is_created_and_retyped_as_manga_and_stored_as_manga() {
        let temp = TempDatabase::open();
        let created = create_library(
            temp.db.clone(),
            NewLibrary {
                library_type: LibraryType::Manga,
                ..new_library("Manga")
            },
        )
        .await
        .unwrap();
        let stored_type = |id| {
            temp.db
                .setup_new_transaction(move |conn| {
                    Ok(conn
                        .query_row(
                            "SELECT library_type FROM library WHERE id = ?1",
                            [id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap())
                })
                .unwrap()
        };

        assert_eq!(created.data.library_type, LibraryType::Manga);
        assert_eq!(stored_type(created.id), "manga");

        let change = |library_type| {
            change_library(
                temp.db.clone(),
                created.id,
                changes(None, Some(library_type), None),
            )
        };
        change(LibraryType::Comics).await.unwrap();
        assert_eq!(stored_type(created.id), "comics");
        let retyped = change(LibraryType::Manga).await.unwrap();
        assert_eq!(retyped.data.library_type, LibraryType::Manga);
        assert_eq!(
            temp.db
                .get_entry::<LibraryMetadata>(created.id)
                .unwrap()
                .library_type,
            LibraryType::Manga
        );
    }

    #[tokio::test]
    async fn a_change_without_fields_a_bad_name_or_an_unknown_library_is_refused() {
        let temp = TempDatabase::open();
        let id = create_library(temp.db.clone(), new_library("Manga"))
            .await
            .unwrap()
            .id;

        let empty = change_library(temp.db.clone(), id, changes(None, None, None)).await;
        assert!(matches!(empty, Err(AppErrors::InvalidRequest)));

        let blank = change_library(temp.db.clone(), id, changes(Some(" "), None, None)).await;
        assert!(matches!(
            blank,
            Err(AppErrors::Library(LibraryError::InvalidName))
        ));

        let unknown =
            change_library(temp.db.clone(), id + 1, changes(Some("Comics"), None, None)).await;
        assert!(matches!(
            unknown,
            Err(AppErrors::Database(DatabaseError::EntryNotFound))
        ));

        let stored = temp.db.get_entry::<LibraryMetadata>(id).unwrap();
        assert_eq!(stored.name, "Manga", "a refused change changes nothing");
    }

    #[tokio::test]
    async fn adding_a_series_twice_links_it_once_and_removing_it_keeps_the_series() {
        let temp = TempDatabase::open();
        let library = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let berserk = add_series_row(&temp, "berserk");

        add_series(temp.db.clone(), library, berserk).await.unwrap();
        add_series(temp.db.clone(), library, berserk).await.unwrap();
        assert_eq!(linked_libraries(&temp, berserk), [library]);

        remove_series(temp.db.clone(), library, berserk)
            .await
            .unwrap();
        assert_eq!(linked_libraries(&temp, berserk), Vec::<i64>::new());
        assert!(temp.db.get_entry::<BookSeriesMetadata>(berserk).is_ok());

        // Removing a series the library doesn't hold changes nothing and is no error.
        remove_series(temp.db.clone(), library, berserk)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn adding_or_removing_with_an_unknown_library_or_series_answers_not_found() {
        let temp = TempDatabase::open();
        let library = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let berserk = add_series_row(&temp, "berserk");

        for (library_id, series_id) in [(library + 1, berserk), (library, berserk + 1)] {
            let added = add_series(temp.db.clone(), library_id, series_id).await;
            let removed = remove_series(temp.db.clone(), library_id, series_id).await;
            for result in [added, removed] {
                assert!(
                    matches!(
                        result,
                        Err(AppErrors::Database(DatabaseError::EntryNotFound))
                    ),
                    "library {library_id}, series {series_id}"
                );
            }
        }
    }

    fn add_book_row(temp: &TempDatabase, series_id: i64, file_path: &str) {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, series) \
                     VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', ?1, ?2)",
                    rusqlite::params![file_path, series_id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
    }

    fn series_changes(
        name: Option<&str>,
        description: Option<Option<&str>>,
        start_release_year: Option<Option<i64>>,
        end_release_year: Option<Option<i64>>,
    ) -> SeriesChanges {
        SeriesChanges {
            name: name.map(String::from),
            description: description.map(|text| text.map(String::from)),
            start_release_year,
            end_release_year,
        }
    }

    #[tokio::test]
    async fn a_series_change_touches_only_its_fields_and_null_clears_them() {
        let temp = TempDatabase::open();
        let berserk = add_series_row(&temp, "berserk");
        add_book_row(&temp, berserk, "/books/Berserk v01.cbz");
        let change = |changes| change_series(temp.db.clone(), berserk, changes);

        let renamed = change(series_changes(Some(" Berserk "), None, None, None))
            .await
            .unwrap();
        assert_eq!(renamed.id, berserk);
        assert_eq!(renamed.data.name, "Berserk");
        assert_eq!(renamed.data.book_count, 1);

        let described = change(series_changes(
            None,
            Some(Some("Dark fantasy")),
            Some(Some(1989)),
            Some(Some(2021)),
        ))
        .await
        .unwrap();
        assert_eq!(described.data.name, "Berserk");
        assert_eq!(described.data.description.as_deref(), Some("Dark fantasy"));
        assert_eq!(described.data.start_release_year, Some(1989));
        assert_eq!(described.data.end_release_year, Some(2021));

        let cleared = change(series_changes(None, Some(None), Some(None), None))
            .await
            .unwrap();
        assert_eq!(cleared.data.description, None);
        assert_eq!(cleared.data.start_release_year, None);
        assert_eq!(
            cleared.data.end_release_year,
            Some(2021),
            "left out, so kept"
        );
    }

    #[tokio::test]
    async fn release_years_outside_1_to_9999_or_out_of_order_are_refused() {
        let temp = TempDatabase::open();
        let berserk = add_series_row(&temp, "berserk");
        let change = |start, end| {
            change_series(
                temp.db.clone(),
                berserk,
                series_changes(None, None, start, end),
            )
        };
        change(Some(Some(1989)), Some(Some(2021))).await.unwrap();

        for (start, end) in [
            (Some(Some(0)), None),
            (Some(Some(-5)), None),
            (None, Some(Some(10_000))),
            (Some(Some(2000)), Some(Some(1999))),
            // A year sent alone is compared with the other one as it is stored.
            (Some(Some(2022)), None),
            (None, Some(Some(1988))),
        ] {
            let result = change(start, end).await;
            assert!(
                matches!(
                    result,
                    Err(AppErrors::Library(LibraryError::InvalidReleaseYears))
                ),
                "{start:?} {end:?}"
            );
        }
        let stored = temp.db.get_entry::<BookSeriesMetadata>(berserk).unwrap();
        assert_eq!(
            (stored.start_release_year, stored.end_release_year),
            (Some(1989), Some(2021)),
            "a refused change changes nothing"
        );

        let same_year = change(Some(Some(1)), Some(Some(1))).await.unwrap();
        assert_eq!(same_year.data.start_release_year, Some(1));
        let last_year = change(None, Some(Some(9999))).await.unwrap();
        assert_eq!(last_year.data.end_release_year, Some(9999));
    }

    #[tokio::test]
    async fn a_series_change_without_fields_a_bad_name_or_an_unknown_series_is_refused() {
        let temp = TempDatabase::open();
        let berserk = add_series_row(&temp, "berserk");
        let change = |series_id, changes| change_series(temp.db.clone(), series_id, changes);

        let empty = change(berserk, series_changes(None, None, None, None)).await;
        assert!(matches!(empty, Err(AppErrors::InvalidRequest)));

        let blank = change(berserk, series_changes(Some("  "), None, None, None)).await;
        assert!(matches!(
            blank,
            Err(AppErrors::Library(LibraryError::InvalidName))
        ));

        let unknown = change(
            berserk + 1,
            series_changes(Some("Monster"), None, None, None),
        )
        .await;
        assert!(matches!(
            unknown,
            Err(AppErrors::Database(DatabaseError::EntryNotFound))
        ));
    }

    #[tokio::test]
    async fn only_a_series_without_books_is_deleted_with_its_links_and_reading_modes() {
        let temp = TempDatabase::open();
        let library = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let berserk = add_series_row(&temp, "berserk");
        let monster = add_series_row(&temp, "monster");
        add_book_row(&temp, berserk, "/books/Berserk v01.cbz");
        for series_id in [berserk, monster] {
            add_series(temp.db.clone(), library, series_id)
                .await
                .unwrap();
        }
        temp.db
            .setup_new_transaction(|conn| {
                let alice = crate::test_support::insert_user(conn, "alice");
                conn.execute(
                    "INSERT INTO reading_modes (user_id, series_id, mode) VALUES (?1, ?2, 'rtl')",
                    [alice, monster],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();

        let refused = delete_series(temp.db.clone(), berserk).await;
        assert!(matches!(
            refused,
            Err(AppErrors::Library(LibraryError::SeriesNotEmpty))
        ));
        assert!(temp.db.get_entry::<BookSeriesMetadata>(berserk).is_ok());
        assert_eq!(linked_libraries(&temp, berserk), [library]);

        delete_series(temp.db.clone(), monster).await.unwrap();
        assert!(temp.db.get_entry::<BookSeriesMetadata>(monster).is_err());
        assert_eq!(linked_libraries(&temp, monster), Vec::<i64>::new());
        let modes: i64 = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .query_row("SELECT COUNT(*) FROM reading_modes", [], |row| row.get(0))
                    .unwrap())
            })
            .unwrap();
        assert_eq!(modes, 0, "the deleted series' reading mode goes with it");

        let again = delete_series(temp.db.clone(), monster).await;
        assert!(matches!(
            again,
            Err(AppErrors::Database(DatabaseError::EntryNotFound))
        ));
    }

    /// A series row with these fields; answers its id.
    fn add_full_series(
        temp: &TempDatabase,
        description: Option<&str>,
        cover: Option<&str>,
        years: (Option<i64>, Option<i64>),
    ) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO series (name, description, cover_image, start_release_year, \
                     end_release_year) VALUES ('Hellboy', ?1, ?2, ?3, ?4)",
                    rusqlite::params![description, cover, years.0, years.1],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }

    /// A series row found by `key` and by each of `extra_keys`; answers its id.
    fn add_keyed_series(temp: &TempDatabase, key: &str, extra_keys: &[&str]) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO series (name, match_key) VALUES (?1, ?1)",
                    [key],
                )
                .unwrap();
                let id = conn.last_insert_rowid();
                for extra in extra_keys {
                    conn.execute(
                        "INSERT INTO series_match_keys (series_id, match_key) VALUES (?1, ?2)",
                        rusqlite::params![id, extra],
                    )
                    .unwrap();
                }
                Ok(id)
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_merge_moves_books_libraries_and_reading_modes_and_deletes_the_merged_series() {
        let temp = TempDatabase::open();
        let manga = create_library(temp.db.clone(), new_library("Manga"))
            .await
            .unwrap()
            .id;
        let favourites = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let merged = add_series_row(&temp, "hellboybprd");
        let kept = add_series_row(&temp, "Hellboy BPRD");
        add_book_row(&temp, merged, "/books/lower/v10.cbz");
        add_book_row(&temp, merged, "/books/lower/v11.cbz");
        add_book_row(&temp, kept, "/books/upper/v10.cbz");
        for (library, series_id) in [(manga, merged), (favourites, merged), (favourites, kept)] {
            add_series(temp.db.clone(), library, series_id)
                .await
                .unwrap();
        }
        let (alice, bob) = temp
            .db
            .setup_new_transaction(|conn| {
                let alice = crate::test_support::insert_user(conn, "alice");
                let bob = crate::test_support::insert_user(conn, "bob");
                for (user_id, series_id, mode) in [
                    (alice, merged, "rtl"),
                    (bob, merged, "rtl"),
                    (bob, kept, "ltr"),
                ] {
                    conn.execute(
                        "INSERT INTO reading_modes (user_id, series_id, mode) VALUES (?1, ?2, ?3)",
                        rusqlite::params![user_id, series_id, mode],
                    )
                    .unwrap();
                }
                Ok((alice, bob))
            })
            .unwrap();

        let answer = merge_series(temp.db.clone(), kept, merged).await.unwrap();

        assert_eq!(answer.id, kept);
        assert_eq!(answer.data.name, "Hellboy BPRD");
        assert_eq!(answer.data.book_count, 3);
        assert_eq!(temp.db.get_books_in_series(kept).unwrap().len(), 3);
        assert!(matches!(
            temp.db.get_entry::<BookSeriesMetadata>(merged),
            Err(DatabaseError::EntryNotFound)
        ));
        assert_eq!(linked_libraries(&temp, kept), [manga, favourites]);
        assert_eq!(linked_libraries(&temp, merged), Vec::<i64>::new());
        let modes: Vec<(i64, i64, String)> = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .prepare(
                        "SELECT user_id, series_id, mode FROM reading_modes \
                         ORDER BY user_id, series_id",
                    )
                    .unwrap()
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap())
            })
            .unwrap();
        assert_eq!(
            modes,
            [
                (alice, kept, "rtl".to_string()),
                (bob, kept, "ltr".to_string()),
            ],
            "alice's mode moved over; bob's own choice for the kept series stayed"
        );
    }

    #[tokio::test]
    async fn a_merge_keeps_the_kept_series_fields_and_fills_only_its_empty_ones() {
        let temp = TempDatabase::open();
        let without_years = add_full_series(&temp, None, Some("kept.png"), (None, None));
        let started = add_full_series(&temp, Some("Kept text"), None, (Some(2000), None));
        for kept in [without_years, started] {
            let merged = add_full_series(
                &temp,
                Some("Dark fantasy"),
                Some("merged.png"),
                (Some(1989), Some(2021)),
            );
            temp.db
                .setup_new_transaction(|conn| {
                    conn.execute("UPDATE series SET name = 'hellboy' WHERE id = ?1", [merged])
                        .unwrap();
                    Ok(())
                })
                .unwrap();
            merge_series(temp.db.clone(), kept, merged).await.unwrap();
        }
        let stored = |id| {
            let series = temp.db.get_entry::<BookSeriesMetadata>(id).unwrap();
            (
                series.name,
                series.description,
                series.cover_image,
                series.start_release_year,
                series.end_release_year,
            )
        };

        assert_eq!(
            stored(without_years),
            (
                "Hellboy".to_string(),
                Some("Dark fantasy".to_string()),
                Some("kept.png".to_string()),
                Some(1989),
                Some(2021)
            )
        );
        assert_eq!(
            stored(started),
            (
                "Hellboy".to_string(),
                Some("Kept text".to_string()),
                Some("merged.png".to_string()),
                Some(2000),
                None
            ),
            "years move only as a pair"
        );
    }

    #[tokio::test]
    async fn a_merge_gives_the_kept_series_each_key_of_the_merged_one_once() {
        let temp = TempDatabase::open();
        let kept = add_keyed_series(&temp, "hellboy bprd", &["hellboybprd"]);
        let merged = add_keyed_series(&temp, "hellboybprd", &["hellboy-bprd", "hellboy bprd"]);

        merge_series(temp.db.clone(), kept, merged).await.unwrap();

        let keys = |series_id| {
            temp.db
                .setup_new_transaction(|conn| {
                    Ok(Database::extra_match_keys_with_connection(conn, series_id)?)
                })
                .unwrap()
        };
        assert_eq!(
            keys(kept),
            ["hellboy-bprd", "hellboybprd"],
            "neither its own key nor one it had twice"
        );
        assert_eq!(keys(merged), Vec::<String>::new());
        let stored = temp.db.get_entry::<BookSeriesMetadata>(kept).unwrap();
        assert_eq!(stored.match_key, "hellboy bprd");
    }

    #[tokio::test]
    async fn merging_a_series_into_itself_or_with_an_unknown_one_is_refused() {
        let temp = TempDatabase::open();
        let kept = add_series_row(&temp, "Berserk");
        let merged = add_series_row(&temp, "berserk");
        add_book_row(&temp, merged, "/books/Berserk v01.cbz");

        let itself = merge_series(temp.db.clone(), merged, merged).await;
        assert!(matches!(itself, Err(AppErrors::InvalidRequest)));
        for (into_id, series_id) in [(kept, merged + 100), (kept + 100, merged)] {
            let unknown = merge_series(temp.db.clone(), into_id, series_id).await;
            assert!(
                matches!(
                    unknown,
                    Err(AppErrors::Database(DatabaseError::EntryNotFound))
                ),
                "into {into_id}, merging {series_id}"
            );
        }

        assert_eq!(
            temp.db.get_books_in_series(merged).unwrap().len(),
            1,
            "a refused merge changes nothing"
        );
        assert!(temp.db.get_entry::<BookSeriesMetadata>(kept).is_ok());
    }

    #[tokio::test]
    async fn every_series_is_listed_by_name_with_its_books_and_libraries() {
        let temp = TempDatabase::open();
        let library = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let monster = add_series_row(&temp, "Monster");
        let berserk = add_series_row(&temp, "berserk");
        add_series(temp.db.clone(), library, berserk).await.unwrap();
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, series) \
                     VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', '/b/v01.cbz', ?1)",
                    [berserk],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();

        let listed = all_series(temp.db.clone()).await.unwrap();

        let rows: Vec<(i64, &str, i64, &[i64])> = listed
            .iter()
            .map(|one| {
                (
                    one.id,
                    one.data.series.name.as_str(),
                    one.data.series.book_count,
                    one.data.library_ids.as_slice(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (berserk, "berserk", 1, [library].as_slice()),
                (monster, "Monster", 0, [].as_slice()),
            ]
        );
    }

    /// A book of the series with this volume and cover file; answers its id.
    fn add_book_with_cover(
        temp: &TempDatabase,
        series_id: i64,
        volume: i64,
        cover: Option<&str>,
    ) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, \
                     volume_number, series, cover_image) \
                     VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', ?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        format!("/books/{series_id} v{volume}.cbz"),
                        volume,
                        series_id,
                        cover
                    ],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_series_shows_the_book_cover_chosen_for_it_until_the_choice_is_deleted() {
        let temp = TempDatabase::open();
        let berserk = add_series_row(&temp, "berserk");
        add_book_with_cover(&temp, berserk, 1, Some("first.png"));
        let second = add_book_with_cover(&temp, berserk, 2, Some("second.png"));
        let shown = || covers::cover_path(temp.db.clone(), covers::CoverKind::Series, berserk);

        choose_series_cover(temp.db.clone(), berserk, second)
            .await
            .unwrap();
        assert_eq!(shown().await.unwrap().as_deref(), Some("second.png"));

        reset_series_cover(temp.db.clone(), berserk).await.unwrap();
        assert_eq!(
            shown().await.unwrap().as_deref(),
            Some("first.png"),
            "the first book's again"
        );
        // Deleting a choice that isn't there changes nothing and is no error.
        reset_series_cover(temp.db.clone(), berserk).await.unwrap();
    }

    #[tokio::test]
    async fn a_series_cover_from_another_series_or_a_book_without_one_is_refused() {
        let temp = TempDatabase::open();
        let berserk = add_series_row(&temp, "berserk");
        let monster = add_series_row(&temp, "monster");
        let chosen = add_book_with_cover(&temp, berserk, 1, Some("first.png"));
        let coverless = add_book_with_cover(&temp, berserk, 2, None);
        let elsewhere = add_book_with_cover(&temp, monster, 1, Some("monster.png"));
        choose_series_cover(temp.db.clone(), berserk, chosen)
            .await
            .unwrap();

        for book in [coverless, elsewhere, elsewhere + 100] {
            let refused = choose_series_cover(temp.db.clone(), berserk, book).await;
            assert!(
                matches!(refused, Err(AppErrors::InvalidRequest)),
                "book {book}"
            );
        }
        for result in [
            choose_series_cover(temp.db.clone(), monster + 100, chosen).await,
            reset_series_cover(temp.db.clone(), monster + 100).await,
        ] {
            assert!(matches!(
                result,
                Err(AppErrors::Database(DatabaseError::EntryNotFound))
            ));
        }
        let stored = temp.db.get_entry::<BookSeriesMetadata>(berserk).unwrap();
        assert_eq!(
            stored.cover_image.as_deref(),
            Some("first.png"),
            "a refused choice changes nothing"
        );
    }

    #[tokio::test]
    async fn a_library_shows_the_cover_a_series_of_it_showed_until_the_choice_is_deleted() {
        let temp = TempDatabase::open();
        let library = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let berserk = add_series_row(&temp, "berserk");
        let monster = add_series_row(&temp, "monster");
        add_book_with_cover(&temp, berserk, 1, Some("berserk.png"));
        add_book_with_cover(&temp, monster, 1, Some("monster.png"));
        let second = add_book_with_cover(&temp, monster, 2, Some("monster-two.png"));
        for series_id in [berserk, monster] {
            add_series(temp.db.clone(), library, series_id)
                .await
                .unwrap();
        }
        let shown = || covers::cover_path(temp.db.clone(), covers::CoverKind::Library, library);

        // Monster has no cover of its own, so the library takes the one it falls back to.
        choose_library_cover(temp.db.clone(), library, monster)
            .await
            .unwrap();
        assert_eq!(shown().await.unwrap().as_deref(), Some("monster.png"));

        choose_series_cover(temp.db.clone(), monster, second)
            .await
            .unwrap();
        assert_eq!(
            shown().await.unwrap().as_deref(),
            Some("monster.png"),
            "the library keeps the picture it was given"
        );
        choose_library_cover(temp.db.clone(), library, monster)
            .await
            .unwrap();
        assert_eq!(shown().await.unwrap().as_deref(), Some("monster-two.png"));

        reset_library_cover(temp.db.clone(), library).await.unwrap();
        assert_eq!(
            shown().await.unwrap().as_deref(),
            Some("berserk.png"),
            "its first series by name again"
        );
    }

    #[tokio::test]
    async fn a_library_cover_from_a_series_it_doesnt_hold_or_one_without_a_cover_is_refused() {
        let temp = TempDatabase::open();
        let library = create_library(temp.db.clone(), new_library("Favourites"))
            .await
            .unwrap()
            .id;
        let coverless = add_series_row(&temp, "empty");
        let elsewhere = add_series_row(&temp, "monster");
        add_book_with_cover(&temp, elsewhere, 1, Some("monster.png"));
        add_series(temp.db.clone(), library, coverless)
            .await
            .unwrap();

        for series_id in [coverless, elsewhere, elsewhere + 100] {
            let refused = choose_library_cover(temp.db.clone(), library, series_id).await;
            assert!(
                matches!(refused, Err(AppErrors::InvalidRequest)),
                "series {series_id}"
            );
        }
        for result in [
            choose_library_cover(temp.db.clone(), library + 1, elsewhere).await,
            reset_library_cover(temp.db.clone(), library + 1).await,
        ] {
            assert!(matches!(
                result,
                Err(AppErrors::Database(DatabaseError::EntryNotFound))
            ));
        }
        let stored = temp.db.get_entry::<LibraryMetadata>(library).unwrap();
        assert_eq!(stored.cover_image, None, "a refused choice changes nothing");
    }
}
