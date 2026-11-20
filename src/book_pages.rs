//! The pages of a book made of images — an image folder's files or a CBZ's image entries — in
//! natural order, numbered from 1. The page routes and the cover picker share this one listing,
//! so a book's cover is always its page 1.

use std::{
    cmp::Ordering,
    fs::{self, File},
    io::{self, Read, Seek},
    path::{Path, PathBuf},
};

use imagesize::{ImageError, ImageResult, ImageSize};
use zip::ZipArchive;

use crate::{error_types::PageError, models::BookFormat};

/// The largest page read out of an archive. Far above a real comic page (a 600-dpi colour scan is
/// about 30 MB), yet small enough that a few pages at once can't exhaust a home server's memory.
pub const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;

/// The first read of an archive entry for its size: holds the header of nearly every page.
const FIRST_SIZE_READ: usize = 16 * 1024;

/// How far into an archive entry its size is looked for. A JPEG's size comes after its EXIF and
/// colour-profile blocks, which seldom reach more than a few hundred kilobytes.
const MAX_SIZE_READ: usize = 1024 * 1024;

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

/// Each page's width and height in pixels, in page order; `None` for a page whose image header
/// can't be read. As the image stores them: EXIF rotation isn't applied.
pub fn page_sizes(format: &BookFormat, path: &Path) -> Result<Vec<Option<ImageSize>>, PageError> {
    // `image` reads a whole JPEG to answer its size; `imagesize` reads only the header.
    match format {
        BookFormat::ImageComic => Ok(folder_pages(path)?
            .iter()
            .map(|page| usable(imagesize::size(page)))
            .collect()),
        BookFormat::Cbz => {
            let mut archive = open_archive(path)?;
            Ok(archive_pages(&archive)
                .iter()
                .map(|(name, _)| entry_size(&mut archive, name))
                .collect())
        }
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

/// The size of the archive entry `name`, from as little of its start as holds the header. A zip
/// entry can't seek past the blocks before it, so the read grows fourfold until the header fits.
fn entry_size(archive: &mut ZipArchive<File>, name: &str) -> Option<ImageSize> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    let mut limit = FIRST_SIZE_READ;
    loop {
        let wanted = limit - bytes.len();
        let read = entry
            .by_ref()
            .take(wanted as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        match imagesize::blob_size(&bytes) {
            // The header runs on past what was read, and the entry holds more.
            Err(ImageError::IoError(error))
                if error.kind() == io::ErrorKind::UnexpectedEof
                    && read == wanted
                    && limit < MAX_SIZE_READ =>
            {
                limit *= 4;
            }
            size => return usable(size),
        }
    }
}

/// A size that was read and has an area: a reader lays a page out by its shape.
fn usable(size: ImageResult<ImageSize>) -> Option<ImageSize> {
    size.ok().filter(|size| size.width > 0 && size.height > 0)
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
    use image::ImageFormat;

    use super::*;
    use crate::test_support::{encoded_image, TempDir};

    fn names(paths: &[PathBuf]) -> Vec<&str> {
        paths.iter().map(|path| file_name(path)).collect()
    }

    fn size(width: usize, height: usize) -> Option<ImageSize> {
        Some(ImageSize { width, height })
    }

    /// Writes `pages` as an image folder and as a CBZ; answers each book's format and path.
    fn as_both_kinds(dir: &TempDir, pages: &[(&str, &[u8])]) -> [(BookFormat, PathBuf); 2] {
        for (name, contents) in pages {
            dir.write(&format!("folder/{name}"), contents);
        }
        let cbz = dir.write_cbz("book.cbz", pages);
        [
            (BookFormat::ImageComic, dir.path().join("folder")),
            (BookFormat::Cbz, cbz),
        ]
    }

    /// `jpeg` with `blocks` colour-profile (APP2) blocks of 64 KiB after its start marker, where a
    /// large ICC profile puts them: before the block that holds the size.
    fn behind_blocks(jpeg: &[u8], blocks: usize) -> Vec<u8> {
        let mut bytes = jpeg[..2].to_vec();
        for _ in 0..blocks {
            // A block's length counts its own two bytes; 0xFFFF is the most it can say.
            bytes.extend_from_slice(&[0xFF, 0xE2, 0xFF, 0xFF]);
            bytes.extend_from_slice(&[0; 0xFFFF - 2]);
        }
        bytes.extend_from_slice(&jpeg[2..]);
        bytes
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
        // The files hold their names, not images, so no page has a size.
        assert_eq!(
            page_sizes(&BookFormat::ImageComic, &book).unwrap(),
            [None, None, None]
        );
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
        assert_eq!(page_sizes(&BookFormat::Cbz, &cbz).unwrap().len(), 4);
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
                page_sizes(&format, &pdf),
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
            page_sizes(&BookFormat::ImageComic, &dir.path().join("gone")),
            Err(PageError::BookMissing)
        ));
        assert!(matches!(
            page_sizes(&BookFormat::Cbz, &dir.path().join("gone.cbz")),
            Err(PageError::BookMissing)
        ));
        assert!(matches!(
            page_sizes(&BookFormat::Cbz, &not_a_zip),
            Err(PageError::Archive(_))
        ));
    }

    #[test]
    fn each_pages_size_is_read_from_its_image_in_page_order() {
        let dir = TempDir::new();
        let png = encoded_image(ImageFormat::Png, 30, 20);
        let jpeg = encoded_image(ImageFormat::Jpeg, 20, 30);
        let webp = encoded_image(ImageFormat::WebP, 7, 5);
        let pages: [(&str, &[u8]); 4] = [
            ("10.png", &png),
            ("2.JPG", &jpeg),
            ("1.webp", &webp),
            ("notes.txt", b"not a page"),
        ];

        for (format, path) in as_both_kinds(&dir, &pages) {
            assert_eq!(
                page_sizes(&format, &path).unwrap(),
                [size(7, 5), size(20, 30), size(30, 20)],
                "{format:?}"
            );
        }
    }

    #[test]
    fn a_jpegs_size_behind_large_blocks_is_found_and_inside_a_cbz_only_up_to_the_cap() {
        let dir = TempDir::new();
        let jpeg = encoded_image(ImageFormat::Jpeg, 40, 60);
        let (deep, too_deep) = (behind_blocks(&jpeg, 2), behind_blocks(&jpeg, 17));
        // `deep` needs the read to grow twice; `too_deep` puts the size past the cap.
        assert!(deep.len() > 4 * FIRST_SIZE_READ && too_deep.len() > MAX_SIZE_READ);
        let pages: [(&str, &[u8]); 2] = [("1.jpg", &deep), ("2.jpg", &too_deep)];

        let [(folder_format, folder), (cbz_format, cbz)] = as_both_kinds(&dir, &pages);

        assert_eq!(page_sizes(&cbz_format, &cbz).unwrap(), [size(40, 60), None]);
        // A file on disk is read by seeking past the blocks, so no cap applies.
        assert_eq!(
            page_sizes(&folder_format, &folder).unwrap(),
            [size(40, 60), size(40, 60)]
        );
    }

    #[test]
    fn a_page_whose_size_cannot_be_read_has_none_and_the_others_keep_theirs() {
        let dir = TempDir::new();
        let png = encoded_image(ImageFormat::Png, 30, 20);
        let jpeg = encoded_image(ImageFormat::Jpeg, 20, 30);
        // A baseline JPEG's size block: its marker, then its length for three colour channels.
        let size_block = jpeg
            .windows(4)
            .position(|bytes| bytes == [0xFF, 0xC0, 0x00, 0x11])
            .expect("the JPEG's size block");
        // A PNG states its width in bytes 16 to 19.
        let mut no_width = png.clone();
        no_width[16..20].fill(0);
        let pages: [(&str, &[u8]); 5] = [
            ("1.jpg", b"not an image"),
            ("2.png", &png),
            ("3.jpg", &jpeg[..size_block]),
            ("4.png", &no_width),
            ("5.webp", b""),
        ];

        for (format, path) in as_both_kinds(&dir, &pages) {
            assert_eq!(
                page_sizes(&format, &path).unwrap(),
                [None, size(30, 20), None, None, None],
                "{format:?}"
            );
        }
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
