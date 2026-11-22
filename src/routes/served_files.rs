//! The file on disk a book or a cover request reads, once its real location — every symlink
//! resolved — is known to lie where the server serves from: the book's directory, or the cover
//! folder.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    db::Database,
    error_types::{AppErrors, DatabaseError},
    models::{BookFormat, BookMetadata, Directory},
    routes::covers::{self, CoverKind},
    scanner::cover_image_retriever::Covers,
};

/// A book's file, or an image folder's folder, at its real location.
#[derive(Debug)]
pub struct BookFile {
    pub format: BookFormat,
    pub path: PathBuf,
}

/// The file of the book `book_id`, once it lies inside the book's directory. A book in no
/// directory — with a warning — and one whose file lies elsewhere or is gone answer `not_found`.
pub async fn book_file(db: Arc<Database>, book_id: i64) -> Result<BookFile, AppErrors> {
    let (book, directory) = db
        .read(move |conn| {
            let book = Database::get_entry_with_connection::<BookMetadata>(conn, book_id)?;
            let directory = match book.directory_id {
                Some(id) => Some(Database::get_entry_with_connection::<Directory>(conn, id)?),
                None => None,
            };
            Ok((book, directory))
        })
        .await?;
    let Some(directory) = directory else {
        tracing::warn!("book {book_id} lies in no directory, so its file is not served");
        return Err(DatabaseError::EntryNotFound.into());
    };
    let path = inside(Path::new(&book.file_path), Path::new(&directory.path))
        .await
        .ok_or(DatabaseError::EntryNotFound)?;
    Ok(BookFile {
        format: book.format,
        path,
    })
}

/// The cover file the `kind` row `id` shows, once it lies inside the cover folder. A row without a
/// cover, and one whose cover lies elsewhere or is gone, answer `not_found`.
pub async fn cover_file(
    db: Arc<Database>,
    covers: &Covers,
    kind: CoverKind,
    id: i64,
) -> Result<PathBuf, AppErrors> {
    let name = covers::cover_path(db, kind, id)
        .await?
        .ok_or(DatabaseError::EntryNotFound)?;
    Ok(inside(&covers.path_of(&name), &covers.folder)
        .await
        .ok_or(DatabaseError::EntryNotFound)?)
}

/// `path` at its real location, when that lies inside `folder`'s; `None` when either is gone, or —
/// with a warning — when it lies elsewhere.
async fn inside(path: &Path, folder: &Path) -> Option<PathBuf> {
    let real = tokio::fs::canonicalize(path).await.ok()?;
    let real_folder = tokio::fs::canonicalize(folder).await.ok()?;
    // Whole components only: `/books-secret` doesn't lie inside `/books`.
    if real.starts_with(&real_folder) {
        return Some(real);
    }
    tracing::warn!("{path:?} leads to {real:?}, outside {real_folder:?}, so it is not served");
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support::{capture_logs, TempDatabase, TempDir};

    /// Runs `future` to its end on this thread, so that `capture_logs` sees what it logs.
    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn directory(temp: &TempDatabase, folder: &Path) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO directories (path) VALUES (?1)",
                    [folder.to_string_lossy()],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }

    /// A book of `format` at `file_path`, in `directory`, with `cover`; answers its id.
    fn book(
        temp: &TempDatabase,
        directory: Option<i64>,
        format: &str,
        file_path: &Path,
        cover: Option<&str>,
    ) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, \
                     directory_id, cover_image) \
                     VALUES ('Book', 'Unknown Author', ?1, 'Unknown', '', ?2, ?3, ?4)",
                    rusqlite::params![format, file_path.to_string_lossy(), directory, cover],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }

    fn is_not_found<T>(result: &Result<T, AppErrors>) -> bool {
        matches!(
            result,
            Err(AppErrors::Database(DatabaseError::EntryNotFound))
        )
    }

    #[tokio::test]
    async fn a_book_inside_its_directory_is_served_from_its_real_location() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        let cbz = files.write("books/Berserk v01.cbz", b"archive");
        files.write("books/Pluto/1.png", b"page");
        let books = directory(&temp, &files.path().join("books"));
        let archive = book(&temp, Some(books), "cbz", &cbz, None);
        // The stored path takes a detour, as a relative scan path could.
        let detour = files.path().join("books/Pluto/../Pluto");
        let folder = book(&temp, Some(books), "image_comic", &detour, None);

        let served = book_file(Arc::clone(&temp.db), archive).await.unwrap();
        assert!(matches!(served.format, BookFormat::Cbz));
        assert_eq!(served.path, fs::canonicalize(&cbz).unwrap());

        let served = book_file(Arc::clone(&temp.db), folder).await.unwrap();
        assert!(matches!(served.format, BookFormat::ImageComic));
        assert_eq!(
            served.path,
            fs::canonicalize(files.path().join("books/Pluto")).unwrap()
        );
    }

    #[test]
    fn a_book_in_no_directory_is_not_served_and_the_log_says_why() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        let pdf = files.write("Downloads/Report.pdf", b"%PDF");
        let unplaced = book(&temp, None, "pdf", &pdf, None);

        let logs = capture_logs(|| {
            assert!(is_not_found(&run(book_file(
                Arc::clone(&temp.db),
                unplaced
            ))));
        });

        assert!(logs.contains(&format!("book {unplaced} lies in no directory")));
    }

    #[test]
    fn a_file_outside_its_directory_is_not_served_however_its_path_is_spelled() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        files.write("books/Berserk v01.pdf", b"%PDF");
        let neighbour = files.write("books-secret/Secret.pdf", b"%PDF");
        let elsewhere = files.write("elsewhere/Other.pdf", b"%PDF");
        let books = directory(&temp, &files.path().join("books"));
        // As text, each of these paths starts with the directory's.
        let climbing = files.path().join("books/../books-secret/Secret.pdf");
        let outside = [neighbour, elsewhere, climbing]
            .map(|path| (book(&temp, Some(books), "pdf", &path, None), path));

        for (id, path) in outside {
            let logs = capture_logs(|| {
                assert!(
                    is_not_found(&run(book_file(Arc::clone(&temp.db), id))),
                    "{path:?}"
                );
            });
            assert!(logs.contains("so it is not served"), "{path:?}: {logs}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_directory_is_served_only_where_it_leads_inside() {
        use std::os::unix::fs::symlink;

        let temp = TempDatabase::open();
        let files = TempDir::new();
        let inside = files.write("books/Berserk v01.pdf", b"%PDF");
        let secret = files.write("elsewhere/Secret.pdf", b"%PDF");
        let books_folder = files.path().join("books");
        symlink(&secret, books_folder.join("Secret.pdf")).unwrap();
        symlink(files.path().join("elsewhere"), books_folder.join("Linked")).unwrap();
        symlink(&inside, books_folder.join("Alias.pdf")).unwrap();
        let books = directory(&temp, &books_folder);
        let [to_file, through_folder, alias] = ["Secret.pdf", "Linked/Secret.pdf", "Alias.pdf"]
            .map(|name| book(&temp, Some(books), "pdf", &books_folder.join(name), None));

        for id in [to_file, through_folder] {
            let logs = capture_logs(|| {
                assert!(is_not_found(&run(book_file(Arc::clone(&temp.db), id))));
            });
            assert!(logs.contains("so it is not served"), "{logs}");
        }
        let served = run(book_file(Arc::clone(&temp.db), alias)).unwrap();
        assert_eq!(served.path, fs::canonicalize(&inside).unwrap());
    }

    #[test]
    fn a_book_whose_file_is_gone_is_not_found_without_a_warning() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        files.write("books/Berserk v01.pdf", b"%PDF");
        let books = directory(&temp, &files.path().join("books"));
        let gone = book(
            &temp,
            Some(books),
            "pdf",
            &files.path().join("books/Gone.pdf"),
            None,
        );

        let logs = capture_logs(|| {
            assert!(is_not_found(&run(book_file(Arc::clone(&temp.db), gone))));
        });

        assert!(!logs.contains("not served"), "{logs}");
    }

    #[test]
    fn a_cover_is_served_only_from_inside_the_cover_folder() {
        let temp = TempDatabase::open();
        let covers = Covers::in_folder(temp.covers.path(), None).unwrap();
        let files = TempDir::new();
        let own = temp.covers.write("own.png", b"cover");
        let outside = files.write("outside.png", b"not a cover");
        let climbing = format!(
            "../{}/outside.png",
            files.path().file_name().unwrap().to_string_lossy()
        );
        let absolute = outside.to_string_lossy().into_owned();
        let cover_of = |name: &str| book(&temp, None, "cbz", &files.path().join(name), Some(name));
        let (shown, climbs, replaces) = (
            cover_of("own.png"),
            cover_of(&climbing),
            cover_of(&absolute),
        );

        let served = run(cover_file(
            Arc::clone(&temp.db),
            &covers,
            CoverKind::Book,
            shown,
        ));
        assert_eq!(served.unwrap(), fs::canonicalize(&own).unwrap());
        for id in [climbs, replaces] {
            let logs = capture_logs(|| {
                let served = run(cover_file(
                    Arc::clone(&temp.db),
                    &covers,
                    CoverKind::Book,
                    id,
                ));
                assert!(is_not_found(&served));
            });
            assert!(logs.contains("so it is not served"), "{logs}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_cover_folder_leading_out_is_not_served() {
        let temp = TempDatabase::open();
        let covers = Covers::in_folder(temp.covers.path(), None).unwrap();
        let files = TempDir::new();
        let outside = files.write("outside.png", b"not a cover");
        std::os::unix::fs::symlink(&outside, covers.folder.join("linked.png")).unwrap();
        let linked = book(
            &temp,
            None,
            "cbz",
            &files.path().join("book.cbz"),
            Some("linked.png"),
        );

        let logs = capture_logs(|| {
            let served = run(cover_file(
                Arc::clone(&temp.db),
                &covers,
                CoverKind::Book,
                linked,
            ));
            assert!(is_not_found(&served));
        });

        assert!(logs.contains("so it is not served"), "{logs}");
    }
}
