//! Static web assets: the dashboard SPAs served from the same binary.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

/// Embedded trace-observatory app.
const INDEX_HTML: &str = include_str!("assets/index.html");

/// Embedded system-monitor app (self-contained: no external requests).
const SYSTEM_HTML: &str = include_str!("assets/system.html");

/// `GET /` — the trace observatory.
async fn index() -> Response {
    html(INDEX_HTML)
}

/// `GET /system` — the system monitor.
async fn system() -> Response {
    html(SYSTEM_HTML)
}

fn html(body: &'static str) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Router for static pages.
pub fn web_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/system", get(system))
        .route("/system/index.html", get(system))
}
