use axum::{
    http::{header, Uri},
    response::{Html, IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;

/// Fallback handler for SPA routing.
///
/// Serves embedded static assets from `frontend/dist/`. For any path that
/// doesn't match a real asset, returns `index.html` so client-side routing
/// works. If `index.html` is somehow missing, returns 404.
pub async fn spa_fallback_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Serve exact asset (CSS, JS, images, fonts)
    if !path.is_empty() {
        if let Some(content) = Assets::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return (
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data,
            )
                .into_response();
        }
    }

    // Fallback to index.html for client-side routing
    if let Some(index) = Assets::get("index.html") {
        return Html(index.data).into_response();
    }

    (axum::http::StatusCode::NOT_FOUND, "404 Not Found").into_response()
}
