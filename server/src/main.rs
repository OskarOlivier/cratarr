mod api;
mod app;
mod config;
mod jellyfin;
mod m3u;
mod nowplaying;
mod playlists;
mod web;

use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

/// `cratarr-server --healthcheck`: for the container's health check (the image has no curl). Exit code 0 when
/// the local server answers its health endpoint with 200.
fn healthcheck() -> i32 {
    let port = std::env::var("CRATARR_BIND")
        .ok()
        .and_then(|b| b.rsplit(':').next().map(str::to_string))
        .unwrap_or_else(|| "8080".into());
    let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{port}")) else { return 1 };
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    if s.write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n").is_err() {
        return 1;
    }
    let mut reply = String::new();
    s.read_to_string(&mut reply).ok();
    i32::from(!reply.starts_with("HTTP/1.1 200") && !reply.starts_with("HTTP/1.0 200"))
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--healthcheck") {
        std::process::exit(healthcheck());
    }
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
