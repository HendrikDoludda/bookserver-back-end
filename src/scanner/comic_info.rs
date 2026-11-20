//! The reading direction a CBZ's or an image folder's `ComicInfo.xml` states, in the `<Manga>`
//! element of ComicRack's schema.

use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

use quick_xml::{events::Event, Reader};
use tracing::warn;

use crate::{
    book_pages,
    error_types::PageError,
    models::{BookFormat, ReadingDirection},
};

const FILE_NAME: &str = "ComicInfo.xml";

/// The largest `ComicInfo.xml` read: a real one is a few kilobytes.
const MAX_BYTES: u64 = 1024 * 1024;

/// The direction the `ComicInfo.xml` of the CBZ or image folder at `path` states; `None` for
/// another format, a book without one, and one that states none or can't be read.
pub fn reading_direction(format: &BookFormat, path: &Path) -> Option<ReadingDirection> {
    stated(format, path).unwrap_or_else(|error| {
        warn!("the ComicInfo.xml of {path:?} can't be read: {error}");
        None
    })
}

fn stated(format: &BookFormat, path: &Path) -> Result<Option<ReadingDirection>, Box<dyn Error>> {
    let bytes = match format {
        BookFormat::Cbz => from_archive(path)?,
        BookFormat::ImageComic => from_folder(path)?,
        BookFormat::Pdf | BookFormat::Epub | BookFormat::None => None,
    };
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let text = std::str::from_utf8(&bytes)?;
    // A byte-order mark, which the XML reader would take for text before the root element.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Ok(direction_in(text)?)
}

/// The file at the archive's root, its name in any case.
fn from_archive(path: &Path) -> Result<Option<Vec<u8>>, PageError> {
    let mut archive = book_pages::open_archive(path)?;
    let Some(name) = archive
        .file_names()
        .find(|name| name.eq_ignore_ascii_case(FILE_NAME))
        .map(str::to_string)
    else {
        return Ok(None);
    };
    book_pages::read_entry(&mut archive, &name, MAX_BYTES).map(Some)
}

/// The file in the folder, its name in any case.
fn from_folder(folder: &Path) -> Result<Option<Vec<u8>>, io::Error> {
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(FILE_NAME));
        if !named || !path.is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        // The size the folder lists can change before the read; the read itself is capped.
        File::open(&path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(io::Error::other(format!("larger than {MAX_BYTES} bytes")));
        }
        return Ok(Some(bytes));
    }
    Ok(None)
}

/// The direction the `<Manga>` element right under `<ComicInfo>` names: `YesAndRightToLeft` is
/// right to left and `No` left to right, in any case; `Yes` and `Unknown` name none.
fn direction_in(xml: &str) -> Result<Option<ReadingDirection>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    // The names of the elements the reader is in, from the root down.
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut manga = String::new();
    loop {
        match reader.read_event()? {
            Event::Start(element) => path.push(element.local_name().as_ref().to_vec()),
            Event::Text(text) if is_manga(&path) => manga.push_str(&text.decode()?),
            Event::End(_) if is_manga(&path) => {
                let value = manga.trim();
                return Ok(if value.eq_ignore_ascii_case("YesAndRightToLeft") {
                    Some(ReadingDirection::Rtl)
                } else if value.eq_ignore_ascii_case("No") {
                    Some(ReadingDirection::Ltr)
                } else {
                    None
                });
            }
            Event::End(_) => {
                path.pop();
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

fn is_manga(path: &[Vec<u8>]) -> bool {
    matches!(path, [root, element] if root == b"ComicInfo" && element == b"Manga")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::test_support::TempDir;

    /// A `ComicInfo.xml` as ComicRack writes one, its `<Manga>` holding `manga`.
    fn comic_info(manga: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Series>Berserk</Series>
  <Manga>{manga}</Manga>
</ComicInfo>"#
        )
    }

    /// The direction a CBZ holding a page and `entry` at `name` states.
    fn in_cbz(dir: &TempDir, name: &str, entry: &str) -> Option<ReadingDirection> {
        static BOOKS: AtomicUsize = AtomicUsize::new(0);
        let cbz = dir.write_cbz(
            &format!("book-{}.cbz", BOOKS.fetch_add(1, Ordering::Relaxed)),
            &[("001.jpg", b"page"), (name, entry.as_bytes())],
        );
        reading_direction(&BookFormat::Cbz, &cbz)
    }

    #[test]
    fn manga_right_to_left_or_no_states_a_direction_and_anything_else_none() {
        let dir = TempDir::new();
        for (manga, expected) in [
            ("YesAndRightToLeft", Some(ReadingDirection::Rtl)),
            ("  yesandrighttoleft\n", Some(ReadingDirection::Rtl)),
            ("No", Some(ReadingDirection::Ltr)),
            ("Yes", None),
            ("Unknown", None),
            ("", None),
        ] {
            assert_eq!(
                in_cbz(&dir, "ComicInfo.xml", &comic_info(manga)),
                expected,
                "{manga:?}"
            );
        }
    }

    #[test]
    fn a_cbz_without_comic_info_or_without_manga_states_none() {
        let dir = TempDir::new();
        let cbz = dir.write_cbz("plain.cbz", &[("001.jpg", b"page")]);
        assert_eq!(reading_direction(&BookFormat::Cbz, &cbz), None);

        assert_eq!(
            in_cbz(
                &dir,
                "ComicInfo.xml",
                "<ComicInfo><Series>Berserk</Series></ComicInfo>"
            ),
            None
        );
    }

    #[test]
    fn the_file_is_found_by_its_name_in_any_case_but_only_at_the_root() {
        let dir = TempDir::new();
        let rtl = comic_info("YesAndRightToLeft");
        for name in ["comicinfo.xml", "COMICINFO.XML"] {
            assert_eq!(
                in_cbz(&dir, name, &rtl),
                Some(ReadingDirection::Rtl),
                "{name}"
            );
        }
        assert_eq!(in_cbz(&dir, "extras/ComicInfo.xml", &rtl), None);
    }

    #[test]
    fn manga_counts_only_right_under_comic_info() {
        let dir = TempDir::new();
        for xml in [
            "<ComicInfo><Pages><Manga>YesAndRightToLeft</Manga></Pages></ComicInfo>",
            "<Other><Manga>YesAndRightToLeft</Manga></Other>",
        ] {
            assert_eq!(in_cbz(&dir, "ComicInfo.xml", xml), None, "{xml}");
        }
    }

    #[test]
    fn an_image_folder_states_its_direction_in_its_own_comic_info() {
        let dir = TempDir::new();
        dir.write("rtl/001.jpg", b"page");
        dir.write(
            "rtl/comicinfo.xml",
            comic_info("YesAndRightToLeft").as_bytes(),
        );
        dir.write("plain/001.jpg", b"page");

        assert_eq!(
            reading_direction(&BookFormat::ImageComic, &dir.path().join("rtl")),
            Some(ReadingDirection::Rtl)
        );
        assert_eq!(
            reading_direction(&BookFormat::ImageComic, &dir.path().join("plain")),
            None
        );
    }

    #[test]
    fn a_byte_order_mark_before_the_xml_is_skipped() {
        let dir = TempDir::new();
        let with_mark = format!("\u{feff}{}", comic_info("No"));
        assert_eq!(
            in_cbz(&dir, "ComicInfo.xml", &with_mark),
            Some(ReadingDirection::Ltr)
        );
    }

    #[test]
    fn a_broken_or_oversized_comic_info_states_none() {
        let dir = TempDir::new();
        assert_eq!(
            in_cbz(
                &dir,
                "ComicInfo.xml",
                "<ComicInfo><Manga>YesAndRightToLeft</Series></ComicInfo>"
            ),
            None,
            "mismatched end tag"
        );
        let cbz = dir.write_cbz(
            "not-utf8.cbz",
            &[
                ("001.jpg", b"page"),
                ("ComicInfo.xml", &[0xff, 0xfe, 0x3c, 0x00]),
            ],
        );
        assert_eq!(reading_direction(&BookFormat::Cbz, &cbz), None, "not UTF-8");

        let padding = format!("<!--{}-->", "x".repeat(MAX_BYTES as usize));
        let oversized = format!("{padding}{}", comic_info("YesAndRightToLeft"));
        assert_eq!(in_cbz(&dir, "ComicInfo.xml", &oversized), None, "in a CBZ");
        dir.write("big/001.jpg", b"page");
        dir.write("big/ComicInfo.xml", oversized.as_bytes());
        assert_eq!(
            reading_direction(&BookFormat::ImageComic, &dir.path().join("big")),
            None,
            "in a folder"
        );
    }

    #[test]
    fn other_formats_and_missing_books_state_none() {
        let dir = TempDir::new();
        let pdf = dir.write("book.pdf", b"%PDF-1.7");
        for format in [BookFormat::Pdf, BookFormat::Epub, BookFormat::None] {
            assert_eq!(reading_direction(&format, &pdf), None);
        }
        assert_eq!(
            reading_direction(&BookFormat::Cbz, &dir.path().join("gone.cbz")),
            None
        );
        assert_eq!(
            reading_direction(&BookFormat::ImageComic, &dir.path().join("gone")),
            None
        );
    }
}
