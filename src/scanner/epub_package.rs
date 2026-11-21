//! The cover an EPUB's package document names, read without the rest of the book.

use std::{path::Path, str};

use percent_encoding::percent_decode_str;
use quick_xml::{
    events::{BytesStart, Event},
    Reader, XmlVersion,
};

use crate::{book_pages, error_types::CoverImageError};

/// Where every EPUB names its package document.
const CONTAINER: &str = "META-INF/container.xml";

/// The largest `container.xml` or package document read. A package document lists every file of
/// the book, yet stays below a megabyte even with thousands of chapters.
const MAX_XML_BYTES: u64 = 8 * 1024 * 1024;

/// An item of the package's manifest whose media type is an image's.
struct Image {
    id: String,
    href: String,
    /// Lowercase: media types ignore case.
    media_type: String,
    /// EPUB 3's mark of the cover: `properties="cover-image"`.
    marked_cover: bool,
}

/// The cover of the EPUB at `path`: the image its package document names, and the extension its
/// media type gives. `None` when the package names none, or one in a format the app can't draw.
pub fn cover_image(path: &Path) -> Result<Option<(Vec<u8>, &'static str)>, CoverImageError> {
    let mut archive = book_pages::open_archive(path)?;
    let container = book_pages::read_entry(&mut archive, CONTAINER, MAX_XML_BYTES)?;
    let Some(package) = package_path(str::from_utf8(&container)?)? else {
        return Err(CoverImageError::EpubWithoutPackage);
    };
    let document = book_pages::read_entry(&mut archive, &package, MAX_XML_BYTES)?;
    let Some(cover) = named_cover(str::from_utf8(&document)?)? else {
        return Ok(None);
    };
    let Some(extension) = extension_for(&cover.media_type) else {
        return Ok(None);
    };
    let Some(entry) = entry_name(&package, &cover.href) else {
        return Err(CoverImageError::EpubCoverHref(cover.href));
    };
    let image = book_pages::read_entry(&mut archive, &entry, book_pages::MAX_PAGE_BYTES)?;
    Ok(Some((image, extension)))
}

/// The package document's path in the archive: the `full-path` of the first `rootfile` that
/// `container.xml` lists.
fn package_path(xml: &str) -> Result<Option<String>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"rootfile" =>
            {
                if let Some(path) = attribute(&element, "full-path")? {
                    return Ok(Some(path));
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// The image the package document names as the cover: EPUB 3's `cover-image` item, else the item
/// EPUB 2's `<meta name="cover">` names, else the first image whose id says cover.
fn named_cover(xml: &str) -> Result<Option<Image>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    let mut images = Vec::new();
    let mut cover_id = None;
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                b"item" => {
                    if let Some(image) = image_item(&element)? {
                        images.push(image);
                    }
                }
                b"meta"
                    if cover_id.is_none()
                        && attribute(&element, "name")?.as_deref() == Some("cover") =>
                {
                    cover_id = attribute(&element, "content")?;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let chosen = images
        .iter()
        .position(|image| image.marked_cover)
        .or_else(|| {
            let id = cover_id.as_deref()?;
            images.iter().position(|image| image.id == id)
        })
        .or_else(|| {
            images
                .iter()
                .position(|image| image.id.to_lowercase().contains("cover"))
        });
    Ok(chosen.map(|index| images.swap_remove(index)))
}

/// The manifest item `element` when it is an image; `None` for any other.
fn image_item(element: &BytesStart) -> Result<Option<Image>, quick_xml::Error> {
    let media_type = attribute(element, "media-type")?
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !media_type.starts_with("image/") {
        return Ok(None);
    }
    let Some(href) = attribute(element, "href")? else {
        return Ok(None);
    };
    let marked_cover = attribute(element, "properties")?.is_some_and(|properties| {
        properties
            .split_ascii_whitespace()
            .any(|property| property == "cover-image")
    });
    Ok(Some(Image {
        id: attribute(element, "id")?.unwrap_or_default(),
        href,
        media_type,
        marked_cover,
    }))
}

/// The value of the attribute `name`, its escapes such as `&amp;` resolved.
fn attribute(element: &BytesStart, name: &str) -> Result<Option<String>, quick_xml::Error> {
    match element.try_get_attribute(name)? {
        Some(found) => Ok(Some(
            found
                .normalized_value(XmlVersion::Implicit1_0)?
                .into_owned(),
        )),
        None => Ok(None),
    }
}

/// The extension of a cover of `media_type`; `None` for any other type, such as SVG, which the
/// app can't draw and which can carry script.
fn extension_for(media_type: &str) -> Option<&'static str> {
    match media_type {
        // `image/jpg` isn't a registered type, but EPUB tools write it.
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// The archive entry that `href`, a URL in the package document at `package`, points to: resolved
/// against the package's folder and percent-decoded. `None` when it climbs out of the archive, or
/// its bytes decode to no text.
fn entry_name(package: &str, href: &str) -> Option<String> {
    let href = percent_decode_str(href).decode_utf8().ok()?;
    let folder = package.rsplit_once('/').map_or("", |(folder, _)| folder);
    let mut parts = Vec::new();
    for part in folder.split('/').chain(href.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error_types::PageError,
        test_support::{package_document, TempDir},
    };

    const COVER: &[u8] = b"the cover";
    const OTHER: &[u8] = b"another picture";

    /// The cover of an EPUB whose package document, at `OEBPS/content.opf`, lists `metadata` and
    /// `manifest`, and which holds `files`.
    fn cover_of(
        metadata: &str,
        manifest: &str,
        files: &[(&str, &[u8])],
    ) -> Option<(Vec<u8>, &'static str)> {
        let dir = TempDir::new();
        let epub = dir.write_epub(
            "Dune.epub",
            "OEBPS/content.opf",
            &package_document(metadata, manifest),
            files,
        );
        cover_image(&epub).unwrap()
    }

    #[test]
    fn epub_3s_cover_image_item_is_the_cover_wherever_the_manifest_lists_it() {
        let cover = cover_of(
            r#"<meta name="cover" content="ornament"/>"#,
            r#"<item id="ornament" href="images/ornament.png" media-type="image/png"/>
               <item id="cover-old" href="images/old.png" media-type="image/png"/>
               <item id="front" href="images/front.jpg" media-type="image/jpeg" properties="svg cover-image"/>"#,
            &[
                ("OEBPS/images/ornament.png", OTHER),
                ("OEBPS/images/old.png", OTHER),
                ("OEBPS/images/front.jpg", COVER),
            ],
        );

        assert_eq!(cover, Some((COVER.to_vec(), "jpg")));
    }

    #[test]
    fn epub_2s_cover_meta_names_the_cover_item_also_with_prefixed_elements() {
        let cover = cover_of(
            r#"<opf:meta name="cover" content="image-2"/>"#,
            r#"<opf:item id="image-1" href="one.png" media-type="image/png"/>
               <opf:item id="cover-old" href="old.png" media-type="image/png"/>
               <opf:item id="image-2" href="two.gif" media-type="image/gif"/>"#,
            &[
                ("OEBPS/one.png", OTHER),
                ("OEBPS/old.png", OTHER),
                ("OEBPS/two.gif", COVER),
            ],
        );

        assert_eq!(cover, Some((COVER.to_vec(), "gif")));
    }

    #[test]
    fn when_nothing_names_a_cover_image_the_first_image_whose_id_says_cover_is_taken() {
        // The meta names the cover page, which is no image, so it names no cover image.
        let cover = cover_of(
            r#"<meta name="cover" content="cover"/>"#,
            r#"<item id="cover" href="cover.xhtml" media-type="application/xhtml+xml"/>
               <item id="logo" href="logo.png" media-type="image/png"/>
               <item id="Book-Cover" href="art.webp" media-type="image/webp"/>
               <item id="cover-back" href="back.png" media-type="image/png"/>"#,
            &[
                ("OEBPS/logo.png", OTHER),
                ("OEBPS/art.webp", COVER),
                ("OEBPS/back.png", OTHER),
            ],
        );

        assert_eq!(cover, Some((COVER.to_vec(), "webp")));
    }

    #[test]
    fn a_book_that_names_no_cover_image_has_none() {
        for (metadata, manifest) in [
            ("", ""),
            (
                "",
                r#"<item id="logo" href="logo.png" media-type="image/png"/>"#,
            ),
            (
                r#"<meta name="cover" content="title"/>"#,
                r#"<item id="title" href="title.xhtml" media-type="application/xhtml+xml"/>
                   <item id="logo" href="logo.png" media-type="image/png"/>"#,
            ),
        ] {
            assert_eq!(
                cover_of(metadata, manifest, &[("OEBPS/logo.png", OTHER)]),
                None,
                "{metadata} {manifest}"
            );
        }
    }

    #[test]
    fn a_named_cover_the_app_cant_draw_gives_none_and_no_other_image_stands_in() {
        for media_type in ["image/svg+xml", "image/tiff"] {
            let manifest = format!(
                r#"<item id="front" href="front.img" media-type="{media_type}" properties="cover-image"/>
                   <item id="cover-jpg" href="fallback.jpg" media-type="image/jpeg"/>"#
            );
            assert_eq!(
                cover_of(
                    "",
                    &manifest,
                    &[("OEBPS/front.img", COVER), ("OEBPS/fallback.jpg", OTHER)]
                ),
                None,
                "{media_type}"
            );
        }
    }

    #[test]
    fn the_media_type_in_any_case_gives_the_extension() {
        for (media_type, extension) in [
            ("image/jpeg", "jpg"),
            ("image/jpg", "jpg"),
            ("IMAGE/PNG", "png"),
            (" image/gif ", "gif"),
            ("image/webp", "webp"),
        ] {
            let manifest = format!(
                r#"<item id="front" href="front" media-type="{media_type}" properties="cover-image"/>"#
            );
            assert_eq!(
                cover_of("", &manifest, &[("OEBPS/front", COVER)]),
                Some((COVER.to_vec(), extension)),
                "{media_type}"
            );
        }
    }

    #[test]
    fn an_href_is_resolved_against_the_package_folder_and_percent_decoded() {
        for (package, href, entry) in [
            (
                "OEBPS/content.opf",
                "images/front.jpg",
                "OEBPS/images/front.jpg",
            ),
            ("content.opf", "front.jpg", "front.jpg"),
            (
                "OEBPS/content.opf",
                "../Images/Front%20Cover.jpg",
                "Images/Front Cover.jpg",
            ),
            (
                "OPS/package/content.opf",
                "./../img/%C3%BCber.jpg",
                "OPS/img/über.jpg",
            ),
        ] {
            assert_eq!(entry_name(package, href).as_deref(), Some(entry), "{href}");
        }
    }

    #[test]
    fn an_href_that_climbs_out_of_the_book_or_decodes_to_no_text_names_no_entry() {
        for (package, href) in [
            ("OEBPS/content.opf", "../../front.jpg"),
            ("content.opf", "../front.jpg"),
            ("OEBPS/content.opf", "front%FF.jpg"),
        ] {
            assert_eq!(entry_name(package, href), None, "{href}");
        }
    }

    #[test]
    fn a_cover_beside_the_package_folder_is_read_by_its_escaped_and_encoded_name() {
        let dir = TempDir::new();
        let epub = dir.write_epub(
            "Dune.epub",
            "OEBPS/content.opf",
            &package_document(
                "",
                r#"<item id="front" href="../Images/Front%20Cover&amp;Back.png" media-type="image/png" properties="cover-image"/>"#,
            ),
            &[("Images/Front Cover&Back.png", COVER)],
        );

        assert_eq!(cover_image(&epub).unwrap(), Some((COVER.to_vec(), "png")));
    }

    #[test]
    fn a_byte_order_mark_and_html_entities_in_the_metadata_dont_hide_the_cover() {
        let document = package_document(
            "<dc:description>Caf&eacute; au lait&nbsp;&amp; more</dc:description>",
            r#"<item id="front" href="front.png" media-type="image/png" properties="cover-image"/>"#,
        );
        let dir = TempDir::new();
        let epub = dir.write_epub(
            "Dune.epub",
            "OEBPS/content.opf",
            &format!("\u{feff}{document}"),
            &[("OEBPS/front.png", COVER)],
        );

        assert_eq!(cover_image(&epub).unwrap(), Some((COVER.to_vec(), "png")));
    }

    #[test]
    fn a_broken_book_is_an_error_that_says_what_broke() {
        let dir = TempDir::new();
        let cover_item = r#"<item id="front" href="front.png" media-type="image/png" properties="cover-image"/>"#;
        let missing_package =
            br#"<container><rootfiles><rootfile full-path="OEBPS/gone.opf"/></rootfiles></container>"#;
        for book in [
            dir.write("not-a-zip.epub", b"plain text"),
            dir.write_cbz(
                "no-container.epub",
                &[("mimetype", b"application/epub+zip")],
            ),
            dir.write_cbz(
                "no-package.epub",
                &[("META-INF/container.xml", missing_package)],
            ),
            dir.write_epub(
                "missing-cover.epub",
                "content.opf",
                &package_document("", cover_item),
                &[],
            ),
        ] {
            assert!(
                matches!(
                    cover_image(&book),
                    Err(CoverImageError::Pages(PageError::Archive(_)))
                ),
                "{book:?}"
            );
        }

        let outside = dir.write_epub(
            "outside.epub",
            "content.opf",
            &package_document(
                "",
                r#"<item id="front" href="../front.png" media-type="image/png" properties="cover-image"/>"#,
            ),
            &[("front.png", COVER)],
        );
        assert!(matches!(
            cover_image(&outside),
            Err(CoverImageError::EpubCoverHref(href)) if href == "../front.png"
        ));

        let no_rootfile = dir.write_cbz(
            "no-rootfile.epub",
            &[(
                "META-INF/container.xml",
                b"<container><rootfiles/></container>",
            )],
        );
        assert!(matches!(
            cover_image(&no_rootfile),
            Err(CoverImageError::EpubWithoutPackage)
        ));

        let unclosed_container = dir.write_cbz(
            "unclosed-container.epub",
            &[(
                "META-INF/container.xml",
                b"<container><rootfiles></container>",
            )],
        );
        let unclosed_package = dir.write_epub(
            "unclosed-package.epub",
            "content.opf",
            &format!("<package><manifest>{cover_item}</package>"),
            &[("front.png", COVER)],
        );
        for book in [unclosed_container, unclosed_package] {
            assert!(
                matches!(cover_image(&book), Err(CoverImageError::EpubXml(_))),
                "{book:?}"
            );
        }

        let not_utf8 = dir.write_cbz(
            "not-utf8.epub",
            &[("META-INF/container.xml", &[0xff, 0xfe, 0x3c, 0x00])],
        );
        assert!(matches!(
            cover_image(&not_utf8),
            Err(CoverImageError::EpubNotUtf8(_))
        ));
    }
}
