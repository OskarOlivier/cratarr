mod api;
mod app;
mod config;
mod guard;
mod index;
mod jellyfin;

use std::time::Duration;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = config::Config::from_env()?;
    let jf = jellyfin::Jellyfin::new(&cfg.jellyfin_url, &cfg.api_key, &cfg.user_id)?;

    // Startup self-check: fail loudly if Jellyfin is unreachable or its API no longer has the shape we rely on.
    let info = jf.public_info().await.context("cannot reach Jellyfin")?;
    tracing::info!("connected to Jellyfin {}", info.version);
    jf.playlists().await.context("Jellyfin's playlist listing did not look as expected")?;

    let bind = cfg.bind.clone();
    let app = app::App::new(cfg, jf);
    app.load_cache().await;

    // Build and maintain the index in the background; the server answers immediately from whatever is cached.
    let syncer = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            if let Err(e) = syncer.sync().await {
                tracing::warn!("index sync failed: {e:#}");
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(&bind).await.with_context(|| format!("cannot bind {bind}"))?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, api::router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
