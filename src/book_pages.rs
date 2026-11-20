//! The pages of a book made of images — an image folder's files or a CBZ's image entries — in
//! natural order, numbered from 1. The page routes and the cover picker share this one listing,
//! so a book's cover is always its page 1.

use std::{
    cmp::Ordering,
    fs::{self, File},
    io::{self, Read, Seek},
    path::{Path, PathBuf},
};

use zip::ZipArchive;

use crate::{error_types::PageError, models::BookFormat};

/// The largest page read out of an archive. Far above a real comic page (a 600-dpi colour scan is
/// about 30 MB), yet small enough that a few pages at once can't exhaust a home server's memory.
pub const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;

/// What a page is served from.
#[derive(Debug, PartialEq)]
pub enum PageContent {
    /// An image folder's file, served from disk.
    File(PathBuf),
    /// A CBZ entry, read out of the archive.
    Bytes {
        bytes: Vec<u8>,
        media_type: &'static str,
    },
}

/// How many pages the book at `path` has.
pub fn page_count(format: &BookFormat, path: &Path) -> Result<usize, PageError> {
    match format {
        BookFormat::ImageComic => Ok(folder_pages(path)?.len()),
        BookFormat::Cbz => Ok(archive_pages(&open_archive(path)?).len()),
        BookFormat::Pdf | BookFormat::Epub | BookFormat::None => Err(PageError::NotAnImageBook),
    }
}

/// Page `number` of the book at `path`, counted from 1. A CBZ entry larger than `max_bytes` is
/// refused.
pub fn page(
    format: &BookFormat,
    path: &Path,
    number: usize,
    max_bytes: u64,
) -> Result<PageContent, PageError> {
    match format {
        BookFormat::ImageComic => Ok(PageContent::File(folder_page(path, number)?)),
        BookFormat::Cbz => {
            let (bytes, media_type) = archive_page(path, number, max_bytes)?;
            Ok(PageContent::Bytes { bytes, media_type })
        }
        BookFormat::Pdf | BookFormat::Epub | BookFormat::None => Err(PageError::NotAnImageBook),
    }
}

/// Page `number` of an image folder: the file's path.
pub fn folder_page(folder: &Path, number: usize) -> Result<PathBuf, PageError> {
    nth_page(folder_pages(folder)?, number)
}

/// Page `number` of a CBZ: the entry's bytes and media type.
pub fn archive_page(
    path: &Path,
    number: usize,
    max_bytes: u64,
) -> Result<(Vec<u8>, &'static str), PageError> {
    let mut archive = open_archive(path)?;
    let (name, media_type) = nth_page(archive_pages(&archive), number)?;
    Ok((read_entry(&mut archive, &name, max_bytes)?, media_type))
}

/// The page images directly in `folder`; its subfolders are books of their own.
fn folder_pages(folder: &Path) -> Result<Vec<PathBuf>, PageError> {
    let entries = fs::read_dir(folder).map_err(missing_or_unreadable)?;
    let mut pages: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && page_media_type(file_name(path)).is_some())
        .collect();
    pages.sort_by(|a, b| natural_order(file_name(a), file_name(b)));
    Ok(pages)
}

/// The page images of a CBZ, each with its media type, in the natural order of their names.
fn archive_pages<R: Read + Seek>(archive: &ZipArchive<R>) -> Vec<(String, &'static str)> {
    let mut pages: Vec<(String, &'static str)> = archive
        .file_names()
        // macOS adds a `__MACOSX` folder of `._` copies to the archives it makes.
        .filter(|name| !name.ends_with('/') && !name.split('/').any(|part| part == "__MACOSX"))
        .filter_map(|name| {
            let media_type = page_media_type(name.rsplit('/').next().unwrap_or_default())?;
            Some((name.to_string(), media_type))
        })
        .collect();
    pages.sort_by(|a, b| natural_order(&a.0, &b.0));
    pages
}

/// The media type of a page image by its file name's extension, in any case; `None` for any
/// other file, and for a hidden one such as macOS's `._` copies.
fn page_media_type(file_name: &str) -> Option<&'static str> {
    if file_name.starts_with('.') {
        return None;
    }
    let extension = Path::new(file_name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

// `2.jpg` before `10.jpg`, and case only decides between names that differ in nothing else.
fn natural_order(a: &str, b: &str) -> Ordering {
    natord::compare_ignore_case(a, b).then_with(|| a.cmp(b))
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

fn nth_page<T>(pages: Vec<T>, number: usize) -> Result<T, PageError> {
    let index = number.checked_sub(1).ok_or(PageError::NoSuchPage)?;
    pages.into_iter().nth(index).ok_or(PageError::NoSuchPage)
}

pub(crate) fn open_archive(path: &Path) -> Result<ZipArchive<File>, PageError> {
    let file = File::open(path).map_err(missing_or_unreadable)?;
    ZipArchive::new(file).map_err(PageError::Archive)
}

/// The entry `name` of `archive`, read whole; one larger than `max_bytes` is refused.
pub(crate) fn read_entry(
    archive: &mut ZipArchive<File>,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, PageError> {
    let too_large = || PageError::PageTooLarge {
        name: name.to_string(),
        max_bytes,
    };
    let entry = archive.by_name(name).map_err(PageError::Archive)?;
    if entry.size() > max_bytes {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    // The size above is what the archive claims; a damaged one can claim less than it holds.
    entry
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(PageError::Unreadable)?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large());
    }
    Ok(bytes)
}

fn missing_or_unreadable(error: io::Error) -> PageError {
    if error.kind() == io::ErrorKind::NotFound {
        PageError::BookMissing
    } else {
        PageError::Unreadable(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn names(paths: &[PathBuf]) -> Vec<&str> {
        paths.iter().map(|path| file_name(path)).collect()
    }

    #[test]
    fn a_folders_pages_are_its_image_files_in_natural_order() {
        let dir = TempDir::new();
        for name in [
            "10.jpg",
            "2.JPG",
            "1.png",
            "notes.txt",
            ".hidden.jpg",
            "._3.jpg",
            "sub/4.jpg",
        ] {
            dir.write(&format!("book/{name}"), name.as_bytes());
        }
        let book = dir.path().join("book");

        assert_eq!(
            names(&folder_pages(&book).unwrap()),
            ["1.png", "2.JPG", "10.jpg"]
        );
        assert_eq!(page_count(&BookFormat::ImageComic, &book).unwrap(), 3);
        assert_eq!(
            page(&BookFormat::ImageComic, &book, 3, MAX_PAGE_BYTES).unwrap(),
            PageContent::File(book.join("10.jpg"))
        );
    }

    #[test]
    fn a_cbzs_pages_are_its_image_entries_in_natural_order() {
        let dir = TempDir::new();
        let cbz = dir.write_cbz(
            "book.cbz",
            &[
                ("ch10/001.jpg", b"ch10-1"),
                ("ch2/002.PNG", b"ch2-2"),
                ("ch2/001.jpg", b"ch2-1"),
                ("__MACOSX/ch2/._001.jpg", b"resource fork"),
                ("ComicInfo.xml", b"<ComicInfo/>"),
                ("cover.webp", b"cover"),
                ("folder.jpg/", b""),
                (".DS_Store", b""),
            ],
        );

        let archive = open_archive(&cbz).unwrap();
        assert_eq!(
            archive_pages(&archive),
            [
                ("ch2/001.jpg".to_string(), "image/jpeg"),
                ("ch2/002.PNG".to_string(), "image/png"),
                ("ch10/001.jpg".to_string(), "image/jpeg"),
                ("cover.webp".to_string(), "image/webp"),
            ]
        );
        assert_eq!(page_count(&BookFormat::Cbz, &cbz).unwrap(), 4);
        assert_eq!(
            page(&BookFormat::Cbz, &cbz, 2, MAX_PAGE_BYTES).unwrap(),
            PageContent::Bytes {
                bytes: b"ch2-2".to_vec(),
                media_type: "image/png"
            }
        );
    }

    #[test]
    fn pages_count_from_1_and_end_at_the_last() {
        let dir = TempDir::new();
        let cbz = dir.write_cbz("book.cbz", &[("1.jpg", b"one"), ("2.jpg", b"two")]);
        dir.write("folder/1.jpg", b"one");
        let folder = dir.path().join("folder");

        assert_eq!(archive_page(&cbz, 1, MAX_PAGE_BYTES).unwrap().0, b"one");
        for number in [0, 3] {
            assert!(
                matches!(
                    archive_page(&cbz, number, MAX_PAGE_BYTES),
                    Err(PageError::NoSuchPage)
                ),
                "{number}"
            );
        }
        assert!(matches!(
            folder_page(&folder, 2),
            Err(PageError::NoSuchPage)
        ));
    }

    #[test]
    fn a_book_that_is_not_made_of_images_has_no_pages() {
        let dir = TempDir::new();
        let pdf = dir.write("book.pdf", b"%PDF-1.7");
        for format in [BookFormat::Pdf, BookFormat::Epub, BookFormat::None] {
            assert!(matches!(
                page_count(&format, &pdf),
                Err(PageError::NotAnImageBook)
            ));
            assert!(matches!(
                page(&format, &pdf, 1, MAX_PAGE_BYTES),
                Err(PageError::NotAnImageBook)
            ));
        }
    }

    #[test]
    fn a_book_gone_from_disk_is_missing_and_a_file_that_is_no_zip_is_an_error() {
        let dir = TempDir::new();
        let not_a_zip = dir.write("book.cbz", b"Rar! renamed to cbz");

        assert!(matches!(
            page_count(&BookFormat::ImageComic, &dir.path().join("gone")),
            Err(PageError::BookMissing)
        ));
        assert!(matches!(
            page_count(&BookFormat::Cbz, &dir.path().join("gone.cbz")),
            Err(PageError::BookMissing)
        ));
        assert!(matches!(
            page_count(&BookFormat::Cbz, &not_a_zip),
            Err(PageError::Archive(_))
        ));
    }

    #[test]
    fn a_page_over_the_cap_is_refused() {
        let dir = TempDir::new();
        let cbz = dir.write_cbz("book.cbz", &[("1.jpg", &[7; 100])]);

        assert!(matches!(
            archive_page(&cbz, 1, 99),
            Err(PageError::PageTooLarge { max_bytes: 99, .. })
        ));
        assert_eq!(archive_page(&cbz, 1, 100).unwrap().0.len(), 100);
    }
}
