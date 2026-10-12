mod api;
mod app;
mod config;
mod covers;
mod jellyfin;
mod m3u;
mod nowplaying;
mod playlists;
mod web;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = config::Config::from_env()?;
    let jf = jellyfin::Jellyfin::new(&cfg.jellyfin_url, &cfg.api_key, &cfg.user_id)?;

    // Startup self-check: fail loudly if Jellyfin is unreachable or the playlist folder cannot be read.
    let info = jf.public_info().await.context("cannot reach Jellyfin")?;
    tracing::info!("connected to Jellyfin {}", info.version);
    jf.activity(1).await.context("Jellyfin's activity log did not look as expected")?;

    let bind = cfg.bind.clone();
    let app = app::App::new(cfg, jf);
    let n = app.playlist_count().await.context("cannot read the playlist folder")?;
    tracing::info!("found {n} playlists");

    let listener = tokio::net::TcpListener::bind(&bind).await.with_context(|| format!("cannot bind {bind}"))?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, api::router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
