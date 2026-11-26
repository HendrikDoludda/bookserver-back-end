//! Which folders the scanner reads: adding one — with a library of its own when asked, and the
//! stored books it adopts — listing them, and removing one with its books.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use rusqlite::Connection;

use crate::{
    data_models::books_model::{DirectorySummary, NewDirectory},
    db::Database,
    error_types::{AppErrors, DatabaseError, DirectoryError},
    models::{
        BookDatabaseColumns, BookMetadata, DatabaseTypes, Directory, LibraryMetadata, LibraryType,
        QuerySeparator, SeriesLibraryConnection, WithId,
    },
    routes::library_management::checked_name,
};

/// The library a folder added with its toggle on gets: the name the request gives, else the
/// folder's own.
struct OwnLibrary {
    name: Option<String>,
    library_type: LibraryType,
}

/// A stored book in no directory whose file lies inside the folder being added.
struct Adoptee {
    id: i64,
    stored_path: String,
    /// Canonical, as the row holds it once the book joins the directory.
    real_path: String,
}

/// Adds the folder at `new.path` and answers it. Refused unless the path names an existing folder
/// that is neither added yet nor inside or around another directory, `cover_folder` or the
/// database's folder. The stored books in no directory whose files lie inside it join it.
pub async fn add_directory(
    db: Arc<Database>,
    cover_folder: PathBuf,
    new: NewDirectory,
) -> Result<WithId<DirectorySummary>, AppErrors> {
    let library = match (new.create_library, new.library_type) {
        (false, _) => None,
        (true, None) => return Err(AppErrors::InvalidRequest),
        (true, Some(library_type)) => Some(OwnLibrary {
            name: new.library_name.as_deref().map(checked_name).transpose()?,
            library_type,
        }),
    };

    let unplaced = Arc::clone(&db)
        .read(|conn| Ok(Database::books_without_directory_with_connection(conn)?))
        .await?;
    let reserved = [cover_folder, db.folder().to_path_buf()];
    let requested = PathBuf::from(new.path);
    // Resolving paths reads the disk: on the blocking pool, and before the transaction, which
    // would hold the write lock all the while.
    let (path, adoptees) = tokio::task::spawn_blocking(move || {
        let path = existing_folder(&requested)?;
        if reserved
            .iter()
            .any(|folder| overlap(Path::new(&path), folder))
        {
            return Err(DirectoryError::Overlaps);
        }
        let adoptees = inside(Path::new(&path), unplaced);
        Ok((path, adoptees))
    })
    .await
    .map_err(|_| DirectoryError::TaskFailed)??;

    db.transact(move |conn| store(conn, path, library, adoptees))
        .await
}

/// Every scanned folder by path, with the library its series join.
pub async fn all_directories(
    db: Arc<Database>,
) -> Result<Vec<WithId<DirectorySummary>>, AppErrors> {
    db.read(|conn| {
        let mut directories = Database::get_all_entries_with_connection::<Directory>(conn)?;
        directories.sort_by(|one, other| one.data.path.cmp(&other.data.path));
        Ok(directories
            .into_iter()
            .map(|directory| WithId {
                id: directory.id,
                data: directory.data.into(),
            })
            .collect())
    })
    .await
}

/// Removes a directory. Its books leave the catalog, and so do the series it leaves without
/// books; its library and the files stay.
pub async fn remove_directory(db: Arc<Database>, directory_id: i64) -> Result<(), AppErrors> {
    db.transact(move |conn| {
        let series = Database::series_of_directory_with_connection(conn, directory_id)?;
        // `books.directory_id` cascades: the directory's books go with it.
        let removed =
            Database::remove_entry_with_connection(conn, DatabaseTypes::Directories, directory_id)?;
        if removed == 0 {
            return Err(DatabaseError::EntryNotFound.into());
        }
        for series_id in series {
            if Database::get_books_in_series_with_connection(conn, series_id)?.is_empty() {
                Database::remove_entry_with_connection(conn, DatabaseTypes::Series, series_id)?;
            }
        }
        Ok(())
    })
    .await
}

/// `requested` with every symlink resolved, when it is an absolute path naming an existing folder.
// A relative path would name another folder whenever the server starts somewhere else.
fn existing_folder(requested: &Path) -> Result<String, DirectoryError> {
    if !requested.is_absolute() {
        return Err(DirectoryError::NotFound);
    }
    let path = fs::canonicalize(requested).map_err(|_| DirectoryError::NotFound)?;
    if !path.is_dir() {
        return Err(DirectoryError::NotFound);
    }
    // The table stores text, so a path that isn't UTF-8 could only be stored altered.
    path.into_os_string()
        .into_string()
        .map_err(|_| DirectoryError::NotFound)
}

/// Whether one folder lies inside the other, or both are the same; compared by whole names, so
/// `/books-old` is not inside `/books`.
fn overlap(one: &Path, other: &Path) -> bool {
    one.starts_with(other) || other.starts_with(one)
}

/// The unplaced books whose files lie inside `folder`, compared canonical: a path scanned through
/// a relative `BOOK_DIRS` such as `../Downloads` resolves against the folder the server runs in,
/// as it did at the scan. A book whose file is gone stays unplaced.
fn inside(folder: &Path, unplaced: Vec<(i64, String)>) -> Vec<Adoptee> {
    unplaced
        .into_iter()
        .filter_map(|(id, stored_path)| {
            let real = fs::canonicalize(&stored_path).ok()?;
            if !real.starts_with(folder) {
                return None;
            }
            let real_path = real.into_os_string().into_string().ok()?;
            Some(Adoptee {
                id,
                stored_path,
                real_path,
            })
        })
        .collect()
}

/// Stores the directory with its own library, when asked for, and the adoptees as its books —
/// unless an added directory is in the way.
fn store(
    conn: &Connection,
    path: String,
    library: Option<OwnLibrary>,
    adoptees: Vec<Adoptee>,
) -> Result<WithId<DirectorySummary>, AppErrors> {
    // Inside the IMMEDIATE transaction, no other request can add a clashing folder between this
    // check and the insert.
    for other in Database::get_all_entries_with_connection::<Directory>(conn)? {
        if other.data.path == path {
            return Err(DirectoryError::Taken.into());
        }
        if overlap(Path::new(&path), Path::new(&other.data.path)) {
            return Err(DirectoryError::Overlaps.into());
        }
    }
    let library_id = match library {
        None => None,
        Some(OwnLibrary { name, library_type }) => {
            let name = match name {
                Some(name) => name,
                None => checked_name(&folder_name(&path))?,
            };
            let library = LibraryMetadata {
                name,
                library_type,
                cover_image: None,
                description: None,
                is_sensitive: false,
            };
            Some(Database::insert_with_connection(conn, &library)?)
        }
    };
    let directory = Directory {
        path,
        library_id,
        created_at: SystemTime::now(),
    };
    let id = Database::insert_with_connection(conn, &directory)?;
    adopt(conn, id, library_id, adoptees)?;
    Ok(WithId {
        id,
        data: directory.into(),
    })
}

/// The last part of a folder's path; the whole path for a drive's root, which has none.
fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(
        || path.to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Moves the adoptees into the directory under their canonical paths and, with `library_id`, puts
/// their series into that library. A book changed since it was read, or whose canonical path
/// another book already has, stays unplaced.
fn adopt(
    conn: &Connection,
    directory_id: i64,
    library_id: Option<i64>,
    adoptees: Vec<Adoptee>,
) -> Result<(), AppErrors> {
    // The library was created in this transaction, so it holds none of their series yet.
    let mut linked = HashSet::new();
    for adoptee in adoptees {
        let book = match Database::get_entry_with_connection::<BookMetadata>(conn, adoptee.id) {
            Ok(book) => book,
            Err(DatabaseError::EntryNotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        if book.directory_id.is_some() || book.file_path != adoptee.stored_path {
            continue;
        }
        if adoptee.real_path != adoptee.stored_path
            && Database::search_for_single_row_with_connection::<BookMetadata>(
                conn,
                &[BookDatabaseColumns::FilePath],
                &[&adoptee.real_path],
                QuerySeparator::And,
            )?
            .is_some()
        {
            continue;
        }
        Database::update_multiple_values_with_connection::<BookMetadata>(
            conn,
            &[
                BookDatabaseColumns::DirectoryId,
                BookDatabaseColumns::FilePath,
            ],
            &[&directory_id, &adoptee.real_path],
            adoptee.id,
        )?;
        if let (Some(library_id), Some(series_id)) = (library_id, book.series) {
            if linked.insert(series_id) {
                Database::insert_with_connection(
                    conn,
                    &SeriesLibraryConnection {
                        series_id,
                        library_id,
                    },
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error_types::LibraryError,
        scanner::cover_image_retriever::Covers,
        test_support::{TempDatabase, TempDir},
    };

    /// `path` canonical, as a directory or an adopted book stores it.
    fn real(path: &Path) -> String {
        fs::canonicalize(path)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap()
    }

    /// A request for `path`; with `library`, the toggle is on with that type and name.
    fn request(path: &Path, library: Option<(LibraryType, Option<&str>)>) -> NewDirectory {
        NewDirectory {
            path: path.to_string_lossy().into_owned(),
            create_library: library.is_some(),
            library_type: library.map(|(library_type, _)| library_type),
            library_name: library.and_then(|(_, name)| name.map(String::from)),
        }
    }

    /// Adds a folder to `temp`'s database, with `temp`'s cover folder.
    async fn add(
        temp: &TempDatabase,
        new: NewDirectory,
    ) -> Result<WithId<DirectorySummary>, AppErrors> {
        let covers = Covers::in_folder(temp.covers.path(), None).unwrap();
        add_directory(Arc::clone(&temp.db), covers.folder, new).await
    }

    fn sql<T>(temp: &TempDatabase, run: impl FnOnce(&Connection) -> T) -> T {
        temp.db.setup_new_transaction(|conn| Ok(run(conn))).unwrap()
    }

    fn series(temp: &TempDatabase, name: &str) -> i64 {
        sql(temp, |conn| {
            conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
                .unwrap();
            conn.last_insert_rowid()
        })
    }

    /// A book of `series` stored at `path` in no directory, as a scan before directories left it.
    fn unplaced_book(temp: &TempDatabase, series: Option<i64>, path: &Path) -> i64 {
        sql(temp, |conn| {
            conn.execute(
                "INSERT INTO books (title, author, format, language, tags, file_path, series) \
                 VALUES ('Book', 'Unknown Author', 'cbz', 'Unknown', '', ?1, ?2)",
                rusqlite::params![path.to_string_lossy(), series],
            )
            .unwrap();
            conn.last_insert_rowid()
        })
    }

    /// A book's directory and path as stored.
    fn placement(temp: &TempDatabase, book: i64) -> (Option<i64>, String) {
        sql(temp, |conn| {
            conn.query_row(
                "SELECT directory_id, file_path FROM books WHERE id = ?1",
                [book],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        })
    }

    fn libraries_of(temp: &TempDatabase, series: i64) -> Vec<i64> {
        sql(temp, |conn| {
            Database::libraries_per_series_with_connection(conn)
                .unwrap()
                .remove(&series)
                .unwrap_or_default()
        })
    }

    fn count(temp: &TempDatabase, table: &str) -> i64 {
        sql(temp, |conn| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
    }

    #[tokio::test]
    async fn a_folder_added_with_its_toggle_on_gets_its_own_library_and_adopts_its_books() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        let berserk = series(&temp, "Berserk");
        let monster = series(&temp, "Monster");
        let saga = series(&temp, "Saga");
        let first = dir.write("Manga/Berserk v01.cbz", b"cbz");
        let second = dir.write("Manga/Berserk v02.cbz", b"cbz");
        dir.write("Manga/Monster/page 1.jpg", b"jpg");
        let outside = dir.write("Comics/Saga v01.cbz", b"cbz");
        // Stored as the scanner found them: on macOS the temp folder is reached through a
        // symlink, so these paths aren't canonical there.
        let books = [
            unplaced_book(&temp, Some(berserk), &first),
            unplaced_book(&temp, Some(berserk), &second),
            unplaced_book(&temp, Some(monster), &dir.path().join("Manga/Monster")),
        ];
        let elsewhere = unplaced_book(&temp, Some(saga), &outside);
        let gone_path = dir.path().join("Manga/Berserk v03.cbz");
        let gone = unplaced_book(&temp, Some(berserk), &gone_path);

        let added = add(
            &temp,
            request(&dir.path().join("Manga"), Some((LibraryType::Comics, None))),
        )
        .await
        .unwrap();

        assert_eq!(added.data.path, real(&dir.path().join("Manga")));
        let library_id = added.data.library_id.expect("a library of its own");
        let library = temp.db.get_entry::<LibraryMetadata>(library_id).unwrap();
        assert_eq!(library.name, "Manga", "named after the folder");
        assert_eq!(library.library_type, LibraryType::Comics);
        let adopted = [
            real(&first),
            real(&second),
            real(&dir.path().join("Manga/Monster")),
        ];
        for (book, path) in books.into_iter().zip(adopted) {
            assert_eq!(placement(&temp, book), (Some(added.id), path));
        }
        assert_eq!(libraries_of(&temp, berserk), [library_id], "linked once");
        assert_eq!(libraries_of(&temp, monster), [library_id]);
        assert_eq!(libraries_of(&temp, saga), Vec::<i64>::new());
        assert_eq!(
            placement(&temp, elsewhere),
            (None, outside.to_string_lossy().into_owned()),
            "a book outside the folder stays unplaced"
        );
        assert_eq!(
            placement(&temp, gone),
            (None, gone_path.to_string_lossy().into_owned()),
            "a book whose file is gone stays unplaced"
        );
    }

    #[tokio::test]
    async fn a_folder_added_with_its_toggle_off_adopts_its_books_into_no_library() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        let berserk = series(&temp, "Berserk");
        let file = dir.write("Manga/Berserk v01.cbz", b"cbz");
        let book = unplaced_book(&temp, Some(berserk), &file);

        let added = add(&temp, request(&dir.path().join("Manga"), None))
            .await
            .unwrap();

        assert_eq!(added.data.library_id, None);
        assert_eq!(count(&temp, "library"), 0, "no library is made");
        assert_eq!(placement(&temp, book), (Some(added.id), real(&file)));
        assert_eq!(libraries_of(&temp, berserk), Vec::<i64>::new());
    }

    #[tokio::test]
    async fn a_given_library_name_is_trimmed_and_a_toggle_without_a_type_or_name_is_refused() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        dir.write("Manga/Berserk v01.cbz", b"cbz");
        let manga = dir.path().join("Manga");

        for (new, expected) in [
            (
                request(&manga, Some((LibraryType::Comics, Some("   ")))),
                "invalid_name",
            ),
            (
                NewDirectory {
                    library_type: None,
                    ..request(&manga, Some((LibraryType::Comics, None)))
                },
                "invalid_request",
            ),
        ] {
            let refused = add(&temp, new).await.unwrap_err();
            assert!(
                matches!(
                    (&refused, expected),
                    (
                        AppErrors::Library(LibraryError::InvalidName),
                        "invalid_name"
                    ) | (AppErrors::InvalidRequest, "invalid_request")
                ),
                "{expected}: {refused:?}"
            );
        }
        assert_eq!(count(&temp, "directories"), 0);
        assert_eq!(count(&temp, "library"), 0);

        let added = add(
            &temp,
            request(&manga, Some((LibraryType::Books, Some("  My Manga \n")))),
        )
        .await
        .unwrap();
        let library = temp
            .db
            .get_entry::<LibraryMetadata>(added.data.library_id.unwrap())
            .unwrap();
        assert_eq!(library.name, "My Manga");
        assert_eq!(library.library_type, LibraryType::Books);
    }

    #[tokio::test]
    async fn a_relative_or_missing_path_or_a_file_is_not_a_folder_to_add() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        let file = dir.write("Manga/Berserk v01.cbz", b"cbz");

        // `src` exists where the tests run, so only the relative-path check refuses it.
        assert!(Path::new("src").is_dir());
        for path in [
            Path::new("src").to_path_buf(),
            Path::new("./src").to_path_buf(),
            dir.path().join("Missing"),
            file,
        ] {
            let refused = add(&temp, request(&path, None)).await.unwrap_err();
            assert!(
                matches!(refused, AppErrors::Directory(DirectoryError::NotFound)),
                "{path:?}: {refused:?}"
            );
        }
        assert_eq!(count(&temp, "directories"), 0);
    }

    #[tokio::test]
    async fn the_same_folder_spelled_another_way_is_taken() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        dir.write("Manga/Berserk v01.cbz", b"cbz");
        dir.write("Comics/Saga v01.cbz", b"cbz");
        add(&temp, request(&dir.path().join("Manga"), None))
            .await
            .unwrap();

        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut spellings = vec![
            dir.path().join("Manga/"),
            dir.path().join("Comics/../Manga"),
            Path::new(&real(dir.path())).join("Manga"),
        ];
        #[cfg(unix)]
        {
            let link = dir.path().join("Link");
            std::os::unix::fs::symlink(dir.path().join("Manga"), &link).unwrap();
            spellings.push(link);
        }
        for path in spellings {
            let refused = add(&temp, request(&path, None)).await.unwrap_err();
            assert!(
                matches!(refused, AppErrors::Directory(DirectoryError::Taken)),
                "{path:?}: {refused:?}"
            );
        }
        assert_eq!(count(&temp, "directories"), 1);
    }

    #[tokio::test]
    async fn a_folder_inside_or_around_another_directory_overlaps_but_a_neighbour_does_not() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        dir.write("Books/Manga/Berserk v01.cbz", b"cbz");
        dir.write("Books-old/Saga v01.cbz", b"cbz");
        add(&temp, request(&dir.path().join("Books"), None))
            .await
            .unwrap();

        for path in [dir.path().join("Books/Manga"), dir.path().to_path_buf()] {
            let refused = add(&temp, request(&path, None)).await.unwrap_err();
            assert!(
                matches!(refused, AppErrors::Directory(DirectoryError::Overlaps)),
                "{path:?}: {refused:?}"
            );
        }
        add(&temp, request(&dir.path().join("Books-old"), None))
            .await
            .expect("only the first part of the name is the same");
        assert_eq!(count(&temp, "directories"), 2);
    }

    #[tokio::test]
    async fn a_folder_inside_or_around_the_cover_folder_or_the_databases_folder_overlaps() {
        let dir = TempDir::new();
        let elsewhere = TempDir::new();
        fs::create_dir_all(dir.path().join("data")).unwrap();
        let db = Arc::new(Database::open(&dir.path().join("data/app_data.sqlite")).unwrap());
        let covers = Covers::in_folder(&elsewhere.path().join("covers"), None).unwrap();
        let add = |path: PathBuf| {
            add_directory(Arc::clone(&db), covers.folder.clone(), request(&path, None))
        };
        for folder in ["data/books", "books"] {
            fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        fs::create_dir_all(elsewhere.path().join("covers/books")).unwrap();

        for path in [
            dir.path().join("data"),
            dir.path().join("data/books"),
            dir.path().to_path_buf(),
            elsewhere.path().join("covers"),
            elsewhere.path().join("covers/books"),
            elsewhere.path().to_path_buf(),
        ] {
            let refused = add(path.clone()).await.unwrap_err();
            assert!(
                matches!(refused, AppErrors::Directory(DirectoryError::Overlaps)),
                "{path:?}: {refused:?}"
            );
        }
        add(dir.path().join("books"))
            .await
            .expect("beside the database's folder");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn of_two_rows_for_one_file_only_the_one_already_canonical_is_adopted() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        let file = dir.write("Manga/Berserk v01.cbz", b"cbz");
        let link = dir.path().join("Link");
        std::os::unix::fs::symlink(dir.path().join("Manga"), &link).unwrap();
        let berserk = series(&temp, "Berserk");
        let through_link = unplaced_book(&temp, Some(berserk), &link.join("Berserk v01.cbz"));
        let canonical = unplaced_book(&temp, Some(berserk), Path::new(&real(&file)));

        let added = add(&temp, request(&dir.path().join("Manga"), None))
            .await
            .unwrap();

        assert_eq!(placement(&temp, canonical), (Some(added.id), real(&file)));
        assert_eq!(
            placement(&temp, through_link),
            (
                None,
                link.join("Berserk v01.cbz").to_string_lossy().into_owned()
            ),
            "left for reconciliation to decide"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_book_stored_relative_to_the_servers_folder_is_found_and_stored_canonical() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        let file = dir.write("Manga/Berserk v01.cbz", b"cbz");
        // As `BOOK_DIRS=../…` stored it: relative to the folder the server runs in.
        let working = std::env::current_dir().unwrap();
        let up = working.components().count() - 1;
        let relative: PathBuf = std::iter::repeat_n(Path::new(".."), up)
            .collect::<PathBuf>()
            .join(file.strip_prefix("/").unwrap());
        let book = unplaced_book(&temp, None, &relative);

        let added = add(&temp, request(&dir.path().join("Manga"), None))
            .await
            .unwrap();

        assert_eq!(placement(&temp, book), (Some(added.id), real(&file)));
    }

    #[tokio::test]
    async fn a_book_changed_or_gone_since_it_was_read_is_not_adopted() {
        let temp = TempDatabase::open();
        let berserk = series(&temp, "Berserk");
        let moved = unplaced_book(&temp, Some(berserk), Path::new("/books/b.cbz"));
        let placed = unplaced_book(&temp, Some(berserk), Path::new("/books/c.cbz"));
        let adoptee = |id: i64, stored: &str| Adoptee {
            id,
            stored_path: stored.to_string(),
            real_path: format!("/real{stored}"),
        };

        sql(&temp, |conn| {
            conn.execute("INSERT INTO directories (path) VALUES ('/other')", [])
                .unwrap();
            let other = conn.last_insert_rowid();
            conn.execute(
                "UPDATE books SET directory_id = ?1 WHERE id = ?2",
                [other, placed],
            )
            .unwrap();
            conn.execute("INSERT INTO directories (path) VALUES ('/real')", [])
                .unwrap();
            let directory = conn.last_insert_rowid();
            adopt(
                conn,
                directory,
                None,
                vec![
                    adoptee(moved, "/books/a.cbz"),
                    adoptee(placed, "/books/c.cbz"),
                    adoptee(placed + 99, "/books/d.cbz"),
                ],
            )
            .unwrap();
        });

        assert_eq!(placement(&temp, moved), (None, "/books/b.cbz".to_string()));
        let (directory, path) = placement(&temp, placed);
        assert!(directory.is_some());
        assert_eq!(path, "/books/c.cbz", "it stays in its own directory");
    }

    #[tokio::test]
    async fn the_directories_are_listed_by_path_with_their_libraries() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        for folder in ["Novels", "Comics", "Manga"] {
            fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        let novels = add(&temp, request(&dir.path().join("Novels"), None))
            .await
            .unwrap();
        let comics = add(
            &temp,
            request(
                &dir.path().join("Comics"),
                Some((LibraryType::Comics, None)),
            ),
        )
        .await
        .unwrap();
        let manga = add(&temp, request(&dir.path().join("Manga"), None))
            .await
            .unwrap();

        let listed = all_directories(Arc::clone(&temp.db)).await.unwrap();

        let found: Vec<(i64, &str, Option<i64>)> = listed
            .iter()
            .map(|one| (one.id, one.data.path.as_str(), one.data.library_id))
            .collect();
        let expected = [&comics, &manga, &novels]
            .map(|one| (one.id, one.data.path.as_str(), one.data.library_id));
        assert_eq!(found, expected);
    }

    #[tokio::test]
    async fn removing_a_directory_removes_its_books_and_the_series_they_leave_empty() {
        let temp = TempDatabase::open();
        let dir = TempDir::new();
        let only_here = series(&temp, "Berserk");
        let shared = series(&temp, "Saga");
        let elsewhere = series(&temp, "Monster");
        let empty = series(&temp, "Empty");
        let files = [
            ("Manga/Berserk v01.cbz", only_here),
            ("Manga/Saga v01.cbz", shared),
            ("Comics/Saga v02.cbz", shared),
            ("Comics/Monster v01.cbz", elsewhere),
        ];
        for (file, series) in files {
            unplaced_book(&temp, Some(series), &dir.write(file, b"cbz"));
        }
        let manga = add(
            &temp,
            request(&dir.path().join("Manga"), Some((LibraryType::Comics, None))),
        )
        .await
        .unwrap();
        let comics = add(&temp, request(&dir.path().join("Comics"), None))
            .await
            .unwrap();
        sql(&temp, |conn| {
            let user = crate::test_support::insert_user(conn, "alice");
            conn.execute(
                "INSERT INTO reading_modes (user_id, series_id, mode) VALUES (?1, ?2, 'rtl')",
                [user, only_here],
            )
            .unwrap();
        });

        remove_directory(Arc::clone(&temp.db), manga.id)
            .await
            .unwrap();

        let paths: Vec<String> = sql(&temp, |conn| {
            conn.prepare("SELECT file_path FROM books ORDER BY file_path")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        });
        assert_eq!(
            paths,
            [
                real(&dir.path().join("Comics/Monster v01.cbz")),
                real(&dir.path().join("Comics/Saga v02.cbz")),
            ]
        );
        let left: Vec<i64> = sql(&temp, |conn| {
            conn.prepare("SELECT id FROM series ORDER BY id")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        });
        assert_eq!(
            left,
            [shared, elsewhere, empty],
            "only the series it emptied go"
        );
        assert_eq!(count(&temp, "reading_modes"), 0, "with their reading modes");
        assert_eq!(count(&temp, "library"), 1, "its library stays");
        let remaining = all_directories(Arc::clone(&temp.db)).await.unwrap();
        assert_eq!(
            remaining.iter().map(|one| one.id).collect::<Vec<_>>(),
            [comics.id]
        );
        assert!(
            dir.path().join("Manga/Berserk v01.cbz").exists(),
            "the files stay"
        );

        let unknown = remove_directory(Arc::clone(&temp.db), manga.id)
            .await
            .unwrap_err();
        assert!(matches!(
            unknown,
            AppErrors::Database(DatabaseError::EntryNotFound)
        ));
    }

    #[test]
    fn folders_overlap_by_whole_names_only() {
        let books = Path::new("/srv/books");
        assert!(overlap(books, books));
        assert!(overlap(books, Path::new("/srv/books/manga")));
        assert!(overlap(Path::new("/srv"), books));
        assert!(!overlap(books, Path::new("/srv/books-old")));
        assert!(!overlap(books, Path::new("/srv/comics")));
    }
}
