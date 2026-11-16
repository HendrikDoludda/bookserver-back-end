use std::path::Path;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use tower::ServiceExt; // for `oneshot`
use tower_http::services::ServeFile;

use crate::error_types::error_response;

/// Serves the file at `path` with HTTP range support. Anything that isn't a file there — nothing,
/// or a folder — answers the JSON `not_found`.
pub async fn serve_file(path: &Path, request: Request<Body>) -> Response<Body> {
    // `ServeFile` streams a folder as if it were a file — a 200 whose body then breaks off — and
    // answers a missing file with an empty 404.
    let is_file = tokio::fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file());
    if !is_file {
        return not_found();
    }
    let Ok(response) = ServeFile::new(path).oneshot(request).await;
    if response.status() == StatusCode::NOT_FOUND {
        // Gone between the check above and the open.
        return not_found();
    }
    response.into_response()
}

fn not_found() -> Response<Body> {
    error_response(StatusCode::NOT_FOUND, "not_found")
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;
    use crate::test_support::TempDir;

    async fn serve(path: &Path, range: Option<&str>) -> (StatusCode, Option<String>, Vec<u8>) {
        let mut request = Request::get("/");
        if let Some(range) = range {
            request = request.header("range", range);
        }
        let response = serve_file(path, request.body(Body::empty()).unwrap()).await;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .map(|value| value.to_str().unwrap().to_string());
        let body = to_bytes(response.into_body(), 1024).await.unwrap().to_vec();
        (status, content_type, body)
    }

    #[tokio::test]
    async fn a_file_is_served_whole_or_by_range_with_its_media_type() {
        let dir = TempDir::new();
        let file = dir.write("book.pdf", b"%PDF-1.7 tiny");

        let (status, content_type, body) = serve(&file, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.as_deref(), Some("application/pdf"));
        assert_eq!(body, b"%PDF-1.7 tiny");

        let (status, _, body) = serve(&file, Some("bytes=0-3")).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(body, b"%PDF");
    }

    #[tokio::test]
    async fn a_missing_file_or_a_folder_answers_the_json_not_found() {
        let dir = TempDir::new();
        dir.write("pages/001.png", b"not really a png");

        for path in [dir.path().join("gone.pdf"), dir.path().join("pages")] {
            let (status, content_type, body) = serve(&path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path:?}");
            assert_eq!(content_type.as_deref(), Some("application/json"));
            assert_eq!(body, br#"{"error":"not_found"}"#, "{path:?}");
        }
    }
}
