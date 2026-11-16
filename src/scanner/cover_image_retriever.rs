use epub_parser::Epub;
use image::ImageFormat;
use pdfium_render::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{book_pages, data_models::models::BookFormat, error_types::CoverImageError};

const COVER_DIR: &str = "./cover_images/";
const DEFAULT_IMAGE: &str = "example-cover.png";

/// Extracts a cover image for the file at `path` into `./cover_images/`, named by
/// the file's content hash, and returns the cover's path.
pub fn get_cover_image(
    path: &Path,
    format: BookFormat,
    hash: &str,
) -> Result<PathBuf, CoverImageError> {
    match format {
        BookFormat::Pdf => get_pdf_cover_image(path, None, hash),
        BookFormat::Cbz => get_cbz_cover_image(path, hash),
        BookFormat::Epub => get_epub_cover_image(path, hash),
        BookFormat::ImageComic => get_image_comic_cover_image(path, hash),
        BookFormat::None => Ok(get_fallback_cover()),
    }
}

fn get_fallback_cover() -> PathBuf {
    Path::new(COVER_DIR).join(DEFAULT_IMAGE)
}

fn get_cbz_cover_image(path: &Path, hash: &str) -> Result<PathBuf, CoverImageError> {
    let (image_data, _) = book_pages::archive_page(path, 1, book_pages::MAX_PAGE_BYTES)?;

    let format =
        image::guess_format(&image_data).map_err(|_| CoverImageError::ImageFormatFailed)?;
    let extension = format.extensions_str().first().copied().unwrap_or("png");

    let final_path = get_cover_output_path(hash, extension)?;

    let result = create_image_at_location(&final_path, &image_data)?;

    Ok(result)
}

// TODO: bundle the PDFium library with the project so PDF covers work on every OS
// (and on macOS without symlinking the dylib).
fn get_pdf_cover_image(
    path: &Path,
    password: Option<&str>,
    hash: &str,
) -> Result<PathBuf, CoverImageError> {
    let pdfium = Pdfium::default();

    let document = pdfium
        .load_pdf_from_file(path, password)
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

    let final_path = get_cover_output_path(hash, "jpg")?;
    if final_path.exists() {
        return Ok(final_path);
    }

    image::DynamicImage::ImageRgb8(image)
        .save_with_format(&final_path, ImageFormat::Jpeg)
        .map_err(|_| CoverImageError::CoverCreationFailed)?;
    Ok(final_path)
}

fn get_epub_cover_image(path: &Path, hash: &str) -> Result<PathBuf, CoverImageError> {
    let epub = Epub::parse(path).map_err(|_| CoverImageError::FileNotFound)?;
    let cover = epub
        .images
        .first()
        .ok_or(CoverImageError::CoverCreationFailed)?;
    let image_bytes = &cover.content;
    let image_type = &cover.media_type;

    // Extract extension from media type
    let extension = image_type
        .split('/')
        .nth(1)
        .ok_or(CoverImageError::CoverCreationFailed)?;

    let final_path = get_cover_output_path(hash, extension)?;
    if final_path.exists() {
        return Ok(final_path);
    }

    let result = create_image_at_location(&final_path, image_bytes)?;
    Ok(result)
}

fn get_image_comic_cover_image(path: &Path, hash: &str) -> Result<PathBuf, CoverImageError> {
    let first = book_pages::folder_page(path, 1)?;

    let extension = first
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("unknown")
        .to_string();

    let final_path = get_cover_output_path(hash, &extension)?;
    if final_path.exists() {
        return Ok(final_path);
    }

    let result = copy_image_to_cover_location(first.as_path(), final_path.as_path())?;

    Ok(result)
}

fn create_image_at_location(path: &Path, data: &[u8]) -> Result<PathBuf, CoverImageError> {
    fs::write(path, data).map_err(|_| CoverImageError::CoverCreationFailed)?;
    Ok(path.to_path_buf())
}

fn copy_image_to_cover_location(
    original_image: &Path,
    final_file: &Path,
) -> Result<PathBuf, CoverImageError> {
    std::fs::copy(original_image, final_file).map_err(|_| CoverImageError::CoverCreationFailed)?;
    Ok(final_file.to_path_buf())
}

fn get_cover_output_path(hash: &str, extension: &str) -> Result<PathBuf, CoverImageError> {
    fs::create_dir_all(COVER_DIR).map_err(|_| CoverImageError::GeneralError)?;

    let final_file = Path::new(COVER_DIR).join(hash).with_extension(extension);
    Ok(final_file)
}
