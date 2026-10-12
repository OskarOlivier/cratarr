//! Serves the built Svelte UI. In release builds the files are embedded in the binary; in debug builds they are
//! read from `../web/dist` on every request, so a `pnpm build` shows up without recompiling the server.

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../web/dist/"]
#[allow_missing = true]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let wanted = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = Assets::get(wanted) {
        return respond(wanted, file);
    }
    // Client-side routes (no file extension) fall back to the app shell.
    if !path.contains('.') {
        if let Some(index) = Assets::get("index.html") {
            return respond("index.html", index);
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "The web UI is not built. Run `pnpm build` in web/ and rebuild the server.",
        )
            .into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

fn respond(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    // Vite fingerprints everything under assets/, so those files can be cached forever; the shell must not be.
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_api_paths_are_404_not_the_app_shell() {
        let resp = serve("/api/does-not-exist".parse().unwrap()).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_static_file_with_extension_is_404() {
        let resp = serve("/nope.js".parse().unwrap()).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
