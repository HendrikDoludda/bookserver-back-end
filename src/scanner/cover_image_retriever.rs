use image::ImageFormat;
use pdfium_render::prelude::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    book_pages, data_models::models::BookFormat, error_types::CoverImageError,
    scanner::epub_package,
};

pub(crate) const DEFAULT_IMAGE: &str = "example-cover.png";

/// The fallback cover, compiled in so a new cover folder can be given it.
const DEFAULT_IMAGE_BYTES: &[u8] = include_bytes!("../../assets/example-cover.png");

/// The extensions a cover can have; an existing cover is looked for under each.
const COVER_EXTENSIONS: [&str; 4] = ["jpg", "png", "webp", "gif"];

/// Where covers are written and served from, and the PDFium library that renders a PDF's first
/// page.
#[derive(Clone)]
pub struct Covers {
    /// The folder every cover file lies in; a row names a file in it.
    pub folder: PathBuf,
    /// `None` without the library: a PDF then gets no cover, but still its row.
    pub pdfium: Option<Arc<Pdfium>>,
}

impl Covers {
    /// Covers in `folder`, which is made if it is missing, and given the fallback cover
    /// ([`DEFAULT_IMAGE`]) if it lacks one; one it has is left as it is.
    // Kept with every symlink resolved: a directory being added is compared with it by real
    // location, which a missing folder doesn't have yet.
    pub fn in_folder(folder: &Path, pdfium: Option<Arc<Pdfium>>) -> io::Result<Self> {
        fs::create_dir_all(folder)?;
        let folder = fs::canonicalize(folder)?;
        if !folder.join(DEFAULT_IMAGE).exists() {
            write_whole(&folder, DEFAULT_IMAGE, DEFAULT_IMAGE_BYTES)
                .map_err(|_| io::Error::other("writing the fallback cover failed"))?;
        }
        Ok(Covers { folder, pdfium })
    }

    /// The cover file a row names.
    pub fn path_of(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }
}

/// Binds the PDFium library once: from the binary's folder, else the working directory, else the
/// system's library path. `None`, with a warning, when none has it; a library that is there but
/// can't be loaded is warned about too.
// `Pdfium::default()` tries only the last two places, and panics when both fail.
pub fn bind_pdfium() -> Option<Pdfium> {
    let exe = std::env::current_exe().ok();
    let working = std::env::current_dir().ok();
    bind_in(&pdfium_folders(exe.as_deref(), working.as_deref())).or_else(bind_system)
}

/// PDFium from the first of `folders` whose library loads; one that is there but doesn't load
/// is warned about.
fn bind_in(folders: &[PathBuf]) -> Option<Pdfium> {
    for folder in folders {
        let path = Pdfium::pdfium_platform_library_name_at_path(folder);
        if !path.is_file() {
            continue;
        }
        match Pdfium::bind_to_library(&path) {
            Ok(bindings) => {
                tracing::info!("PDFium loaded from {}", path.display());
                return Some(Pdfium::new(bindings));
            }
            Err(error) => tracing::warn!("PDFium at {} can't be loaded: {error:?}", path.display()),
        }
    }
    None
}

fn bind_system() -> Option<Pdfium> {
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            tracing::info!("PDFium loaded from the system's library path");
            Some(Pdfium::new(bindings))
        }
        Err(error) => {
            tracing::warn!("The PDFium library can't be loaded, so PDFs get no cover: {error:?}");
            None
        }
    }
}

/// The folders PDFium is looked for in before the system's library path, in order: the one the
/// binary lies in, then the working directory — each once.
fn pdfium_folders(exe: Option<&Path>, working: Option<&Path>) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for folder in [exe.and_then(Path::parent), working].into_iter().flatten() {
        if !folders.iter().any(|known: &PathBuf| known == folder) {
            folders.push(folder.to_path_buf());
        }
    }
    folders
}

/// The file name of the cover for the book at `path`: one already in the cover folder under the
/// file's hash, else one extracted now. `None` when the book can't have one — no hash, a PDF
/// without PDFium, no picture to take — or when extracting fails, which is logged.
pub fn cover_for(
    covers: &Covers,
    path: &Path,
    format: &BookFormat,
    hash: Option<&str>,
) -> Option<String> {
    // Covers are named by the hash: without one, every such book would share one file.
    let hash = hash.filter(|hash| !hash.is_empty())?;
    match extract(covers, path, format, hash) {
        Ok(name) => name,
        Err(error) => {
            tracing::error!("Failed cover creation for path {path:?}: {error}");
            None
        }
    }
}

fn extract(
    covers: &Covers,
    path: &Path,
    format: &BookFormat,
    hash: &str,
) -> Result<Option<String>, CoverImageError> {
    if let Some(existing) = existing_cover(&covers.folder, hash) {
        return Ok(Some(existing));
    }
    match format {
        BookFormat::Pdf => match &covers.pdfium {
            Some(pdfium) => pdf_cover(pdfium, &covers.folder, path, hash).map(Some),
            None => Ok(None),
        },
        BookFormat::Cbz => cbz_cover(&covers.folder, path, hash).map(Some),
        BookFormat::Epub => epub_cover(&covers.folder, path, hash),
        BookFormat::ImageComic => folder_cover(&covers.folder, path, hash).map(Some),
        BookFormat::None => Ok(Some(DEFAULT_IMAGE.to_string())),
    }
}

/// The name of a cover already extracted for `hash`, under any extension.
fn existing_cover(folder: &Path, hash: &str) -> Option<String> {
    COVER_EXTENSIONS
        .iter()
        .map(|extension| format!("{hash}.{extension}"))
        .find(|name| folder.join(name).is_file())
}

fn cbz_cover(folder: &Path, path: &Path, hash: &str) -> Result<String, CoverImageError> {
    let (image_data, _) = book_pages::archive_page(path, 1, book_pages::MAX_PAGE_BYTES)?;
    let extension = image_extension(&image_data)?;
    write_cover(folder, hash, extension, &image_data)
}

fn pdf_cover(
    pdfium: &Pdfium,
    folder: &Path,
    path: &Path,
    hash: &str,
) -> Result<String, CoverImageError> {
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|_| CoverImageError::PdfToDocumentError)?;

    let render_config = PdfRenderConfig::new()
        .set_target_width(1000)
        .set_maximum_height(2000)
        .rotate_if_landscape(PdfPageRenderRotation::Degrees90, true);

    let page = document
        .pages()
        .get(0)
        .map_err(|_| CoverImageError::PageCouldNotBeRetrieved)?;

    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|_| CoverImageError::PageToImageConversionFailed)?;

    let image = bitmap
        .as_image()
        .map_err(|_| CoverImageError::CoverCreationFailed)?
        .into_rgb8();

    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut io::Cursor::new(&mut jpeg), ImageFormat::Jpeg)
        .map_err(|_| CoverImageError::CoverCreationFailed)?;
    write_cover(folder, hash, "jpg", &jpeg)
}

/// The cover of an EPUB: the image its package document names. `None` when it names none the app
/// can draw.
fn epub_cover(folder: &Path, path: &Path, hash: &str) -> Result<Option<String>, CoverImageError> {
    let Some((image, extension)) = epub_package::cover_image(path)? else {
        return Ok(None);
    };
    write_cover(folder, hash, extension, &image).map(Some)
}

/// The cover of an image folder: its page 1, once its bytes show it is a picture.
fn folder_cover(folder: &Path, path: &Path, hash: &str) -> Result<String, CoverImageError> {
    let first = book_pages::folder_page(path, 1)?;
    let image = book_pages::read_page_file(&first, book_pages::MAX_PAGE_BYTES)?;
    let extension = image_extension(&image)?;
    write_cover(folder, hash, extension, &image)
}

/// The extension of the picture `data` starts with, by its content, whatever its file is called.
fn image_extension(data: &[u8]) -> Result<&'static str, CoverImageError> {
    let format = image::guess_format(data).map_err(|_| CoverImageError::ImageFormatFailed)?;
    Ok(format.extensions_str().first().copied().unwrap_or("png"))
}

fn write_cover(
    folder: &Path,
    hash: &str,
    extension: &str,
    data: &[u8],
) -> Result<String, CoverImageError> {
    let name = format!("{hash}.{extension}");
    write_whole(folder, &name, data)?;
    Ok(name)
}

/// Writes `data` to the file `name` in `folder`, under a temporary name first.
fn write_whole(folder: &Path, name: &str, data: &[u8]) -> Result<(), CoverImageError> {
    create_folder(folder)?;
    // Renamed only once whole: a write cut short must not leave a file a later scan takes for a
    // finished cover. Two scans may write the same cover at once, so each has its own temp name.
    let partial = folder.join(format!(".{name}.{}.partial", Uuid::new_v4()));
    let written = fs::write(&partial, data).and_then(|()| fs::rename(&partial, folder.join(name)));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
        return Err(CoverImageError::CoverCreationFailed);
    }
    Ok(())
}

fn create_folder(folder: &Path) -> Result<(), CoverImageError> {
    fs::create_dir_all(folder).map_err(|_| CoverImageError::GeneralError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{capture_logs, encoded_image, package_document, TempDir};

    const HASH: &str = "0123abcd";

    fn covers_in(dir: &TempDir) -> Covers {
        Covers {
            folder: dir.path().join("covers"),
            pdfium: None,
        }
    }

    #[test]
    fn a_cbz_cover_is_its_first_page_named_by_the_hash_in_the_cover_folder() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let png = encoded_image(ImageFormat::Png, 2, 3);
        let cbz = books.write_cbz("Berserk v01.cbz", &[("001.png", &png)]);

        let name = cover_for(&covers, &cbz, &BookFormat::Cbz, Some(HASH));

        assert_eq!(name.as_deref(), Some("0123abcd.png"));
        assert_eq!(fs::read(covers.path_of("0123abcd.png")).unwrap(), png);
    }

    fn files_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn pdfium_is_looked_for_beside_the_binary_first_then_in_the_working_directory() {
        let folders = pdfium_folders(
            Some(Path::new("/opt/bookserver/bookserver_backend")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(
            folders,
            [PathBuf::from("/opt/bookserver"), PathBuf::from("/home/me")]
        );
    }

    #[test]
    fn a_binary_started_in_its_own_folder_looks_there_once() {
        let folders = pdfium_folders(
            Some(Path::new("/opt/bookserver/bookserver_backend")),
            Some(Path::new("/opt/bookserver")),
        );
        assert_eq!(folders, [PathBuf::from("/opt/bookserver")]);
    }

    #[test]
    fn an_unknown_binary_or_working_directory_is_left_out() {
        let working = Path::new("/home/me");
        let binary = Path::new("/opt/bookserver/bookserver_backend");
        assert_eq!(pdfium_folders(None, Some(working)), [working.to_path_buf()]);
        assert_eq!(
            pdfium_folders(Some(binary), None),
            [PathBuf::from("/opt/bookserver")]
        );
        assert!(pdfium_folders(None, None).is_empty());
    }

    #[test]
    fn a_folder_without_pdfium_is_passed_over_without_a_warning() {
        let empty = TempDir::new();
        fs::create_dir_all(empty.path()).unwrap();

        let mut bound = None;
        let logs = capture_logs(|| bound = bind_in(&[empty.path().to_path_buf()]));

        assert!(bound.is_none());
        assert!(!logs.contains("PDFium"), "{logs}");
    }

    #[test]
    fn a_pdfium_library_that_does_not_load_is_warned_about_and_passed_over() {
        let broken = TempDir::new();
        let library = broken.write(
            &Pdfium::pdfium_platform_library_name().to_string_lossy(),
            b"not a library",
        );

        let mut bound = None;
        let logs = capture_logs(|| bound = bind_in(&[broken.path().to_path_buf()]));

        assert!(bound.is_none());
        assert!(
            logs.contains(&format!("PDFium at {} can't be loaded", library.display())),
            "{logs}"
        );
    }

    #[test]
    fn a_new_cover_folder_gets_the_fallback_cover_and_nothing_else() {
        let dir = TempDir::new();

        let covers = Covers::in_folder(&dir.path().join("covers"), None).unwrap();

        assert_eq!(files_in(&covers.folder), [DEFAULT_IMAGE]);
        assert_eq!(
            fs::read(covers.path_of(DEFAULT_IMAGE)).unwrap(),
            DEFAULT_IMAGE_BYTES
        );
        assert!(
            image::load_from_memory(DEFAULT_IMAGE_BYTES).is_ok(),
            "the compiled-in fallback is a readable image"
        );
    }

    #[test]
    fn a_fallback_cover_the_folder_has_is_left_as_it_is() {
        let dir = TempDir::new();
        dir.write(&format!("covers/{DEFAULT_IMAGE}"), b"one of its own");

        let covers = Covers::in_folder(&dir.path().join("covers"), None).unwrap();

        assert_eq!(
            fs::read(covers.path_of(DEFAULT_IMAGE)).unwrap(),
            b"one of its own"
        );
        assert_eq!(files_in(&covers.folder), [DEFAULT_IMAGE]);
    }

    #[test]
    fn a_cover_is_renamed_into_place_once_whole_and_leaves_no_temporary_file() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let png = encoded_image(ImageFormat::Png, 2, 3);
        books.write("Pluto/001.png", &png);
        let cbz = books.write_cbz("Berserk v01.cbz", &[("001.png", &png)]);

        cover_for(&covers, &cbz, &BookFormat::Cbz, Some(HASH));
        cover_for(
            &covers,
            &books.path().join("Pluto"),
            &BookFormat::ImageComic,
            Some("4567cdef"),
        );

        assert_eq!(files_in(&covers.folder), ["0123abcd.png", "4567cdef.png"]);
    }

    #[test]
    fn a_temporary_file_left_by_a_write_cut_short_is_not_taken_for_a_cover() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        fs::create_dir_all(&covers.folder).unwrap();
        let torn = covers.folder.join(".0123abcd.png.1234.partial");
        fs::write(&torn, b"half a pic").unwrap();
        let png = encoded_image(ImageFormat::Png, 2, 3);
        let cbz = books.write_cbz("Berserk v01.cbz", &[("001.png", &png)]);

        let name = cover_for(&covers, &cbz, &BookFormat::Cbz, Some(HASH));

        assert_eq!(name.as_deref(), Some("0123abcd.png"));
        assert_eq!(fs::read(covers.path_of("0123abcd.png")).unwrap(), png);
    }

    #[test]
    fn a_folder_cover_is_named_by_what_its_first_page_holds_not_by_its_file_name() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let png = encoded_image(ImageFormat::Png, 2, 3);
        books.write("Pluto/001.jpg", &png);
        books.write("Pluto/002.jpg", &encoded_image(ImageFormat::Jpeg, 2, 3));

        let name = cover_for(
            &covers,
            &books.path().join("Pluto"),
            &BookFormat::ImageComic,
            Some(HASH),
        );

        assert_eq!(name.as_deref(), Some("0123abcd.png"));
        assert_eq!(fs::read(covers.path_of("0123abcd.png")).unwrap(), png);
    }

    #[test]
    fn a_first_page_that_is_no_picture_gives_no_cover() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        books.write("Pluto/001.jpg", b"not a picture");
        let cbz = books.write_cbz("Monster v01.cbz", &[("001.jpg", b"not a picture")]);

        for (path, format) in [
            (books.path().join("Pluto"), BookFormat::ImageComic),
            (cbz, BookFormat::Cbz),
        ] {
            assert_eq!(cover_for(&covers, &path, &format, Some(HASH)), None);
        }
        assert!(!covers.folder.exists(), "nothing is written");
    }

    #[test]
    fn an_existing_cover_is_found_before_anything_is_read_or_written() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        fs::create_dir_all(&covers.folder).unwrap();
        fs::write(covers.path_of("0123abcd.jpg"), b"extracted before").unwrap();
        // The archive is gone, so reading it would fail: the cover can only come from the folder.
        let gone = books.path().join("gone.cbz");

        let name = cover_for(&covers, &gone, &BookFormat::Cbz, Some(HASH));

        assert_eq!(name.as_deref(), Some("0123abcd.jpg"));
        assert_eq!(
            fs::read(covers.path_of("0123abcd.jpg")).unwrap(),
            b"extracted before"
        );
    }

    #[test]
    fn without_a_hash_there_is_no_cover_and_no_file_beside_the_folder() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let png = encoded_image(ImageFormat::Png, 2, 3);
        let cbz = books.write_cbz("Berserk v01.cbz", &[("001.png", &png)]);

        for hash in [None, Some("")] {
            assert_eq!(cover_for(&covers, &cbz, &BookFormat::Cbz, hash), None);
        }
        assert!(!covers.folder.exists());
        assert!(!books.path().join("covers.png").exists());
    }

    #[test]
    fn an_epub_cover_is_the_image_its_package_names_stored_under_the_hash() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let epub = books.write_epub(
            "Dune.epub",
            "OEBPS/content.opf",
            &package_document(
                "",
                r#"<item id="front" href="images/front.jpg" media-type="image/jpeg" properties="cover-image"/>"#,
            ),
            &[("OEBPS/images/front.jpg", b"the cover")],
        );

        let name = cover_for(&covers, &epub, &BookFormat::Epub, Some(HASH));

        assert_eq!(name.as_deref(), Some("0123abcd.jpg"));
        assert_eq!(
            fs::read(covers.path_of("0123abcd.jpg")).unwrap(),
            b"the cover"
        );
    }

    #[test]
    fn an_epub_without_a_cover_the_app_can_draw_gets_none_and_nothing_is_written() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let svg = books.write_epub(
            "Dune.epub",
            "OEBPS/content.opf",
            &package_document(
                "",
                r#"<item id="front" href="front.svg" media-type="image/svg+xml" properties="cover-image"/>"#,
            ),
            &[("OEBPS/front.svg", b"<svg/>")],
        );
        let no_image = books.write_epub("Emma.epub", "content.opf", &package_document("", ""), &[]);

        for epub in [svg, no_image] {
            assert_eq!(
                cover_for(&covers, &epub, &BookFormat::Epub, Some(HASH)),
                None
            );
        }
        assert!(!covers.folder.exists(), "nothing is written");
    }

    #[test]
    fn a_pdf_without_pdfium_gets_no_cover() {
        let books = TempDir::new();
        let covers = covers_in(&books);
        let pdf = books.write("Berserk v01.pdf", b"%PDF-1.7 tiny");

        assert_eq!(cover_for(&covers, &pdf, &BookFormat::Pdf, Some(HASH)), None);
        assert!(!covers.folder.exists());
    }
}
