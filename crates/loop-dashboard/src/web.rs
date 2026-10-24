//! Static web assets: the dashboard SPA served from the same binary.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

/// Embedded single-page app.
const INDEX_HTML: &str = include_str!("assets/index.html");

/// `GET /` — the dashboard.
async fn index() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
        .into_response()
}

/// Router for static pages.
pub fn web_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
}
