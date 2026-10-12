//! HTTP API.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use crate::app::{App, StateView, ToggleError};

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(state))
        .route("/api/playlists/{id}/toggle", post(toggle))
        .with_state(app)
}

async fn health(State(app): State<Arc<App>>) -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "playlists": app.playlist_count().await }))
}

async fn state(State(app): State<Arc<App>>) -> Result<Json<StateView>, ApiError> {
    Ok(Json(app.state().await.map_err(ToggleError::from)?))
}

async fn toggle(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<StateView>, ApiError> {
    Ok(Json(app.toggle(&id).await?))
}

pub struct ApiError(ToggleError);

impl From<ToggleError> for ApiError {
    fn from(e: ToggleError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ToggleError::NothingPlaying | ToggleError::Incoming | ToggleError::NotIndexedYet => StatusCode::CONFLICT,
            ToggleError::UnknownPlaylist => StatusCode::NOT_FOUND,
            ToggleError::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        if matches!(self.0, ToggleError::Upstream(_)) {
            tracing::error!("{:#}", self.0);
        }
        (status, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}
