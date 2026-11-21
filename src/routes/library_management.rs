//! What an admin changes about libraries: creating and editing them, and which series they hold.

use std::sync::Arc;

use rusqlite::{Connection, ToSql};

use crate::{
    data_models::books_model::{
        LibraryChanges, LibrarySummary, NewLibrary, SeriesListing, SeriesSummary,
    },
    db::Database,
    error_types::{AppErrors, DatabaseError, LibraryError},
    models::{
        BookSeriesMetadata, LibraryDatabaseColumns, LibraryMetadata, SeriesLibraryConnection,
        WithId,
    },
};

/// Room for any real title, while a pasted page of text is still refused.
const MAX_NAME_CHARS: usize = 200;

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
    } = changes;
    if name.is_none() && library_type.is_none() && description.is_none() {
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

/// `DatabaseError::EntryNotFound` unless both the library and the series exist.
fn existing(conn: &Connection, library_id: i64, series_id: i64) -> Result<(), DatabaseError> {
    Database::get_entry_with_connection::<LibraryMetadata>(conn, library_id)?;
    Database::get_entry_with_connection::<BookSeriesMetadata>(conn, series_id)?;
    Ok(())
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
}
