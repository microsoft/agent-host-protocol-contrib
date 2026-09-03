//! Local static Vite hosting; reusable AHP crates remain transport-only.

use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::Html;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

const INDEX_FILE: &str = "index.html";
const MISSING_FRONTEND_PAGE: &str = r#"<!doctype html>
<html lang="en">
  <head><meta charset="utf-8"><title>AHP chat frontend is not built</title></head>
  <body>
    <h1>AHP chat frontend is not built</h1>
    <p>The AHP WebSocket endpoint remains available at <code>/ahp</code>.</p>
    <p>Build the frontend from the repository root:</p>
    <pre>cd samples/web/ahp-chat
npm ci
npm run build</pre>
  </body>
</html>"#;

/// Frontend availability known when the host starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebStatus {
    /// A non-empty Vite `dist/index.html` is served.
    Serving,
    /// Static hosting was intentionally disabled.
    Disabled,
    /// The configured frontend root lacks a usable Vite build.
    MissingBuild,
}

/// Static router and the availability status used by CLI output.
pub(crate) struct WebComposition {
    pub(crate) router: Router,
    pub(crate) status: WebStatus,
}

/// Builds a static SPA fallback router or a 503 page with build instructions.
pub(crate) fn compose(web_root: Option<PathBuf>) -> WebComposition {
    let Some(web_root) = web_root else {
        return WebComposition {
            router: Router::new().fallback(missing_frontend),
            status: WebStatus::Disabled,
        };
    };
    if !is_built(&web_root) {
        return WebComposition {
            router: Router::new().fallback(missing_frontend),
            status: WebStatus::MissingBuild,
        };
    }
    let index = web_root.join(INDEX_FILE);
    WebComposition {
        router: Router::new()
            .fallback_service(ServeDir::new(web_root).fallback(ServeFile::new(index))),
        status: WebStatus::Serving,
    }
}

/// Returns whether `root` contains a usable Vite entry document.
pub(crate) fn is_built(root: &std::path::Path) -> bool {
    root.join(INDEX_FILE)
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

#[cfg(test)]
pub(crate) fn router(web_root: Option<PathBuf>) -> Router {
    compose(web_root).router
}

async fn missing_frontend() -> (StatusCode, Html<&'static str>) {
    (StatusCode::SERVICE_UNAVAILABLE, Html(MISSING_FRONTEND_PAGE))
}
