//! Application logic: works out what is playing and which playlist files contain it, and decides what a tap does.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::jellyfin::{Jellyfin, PlayingItem, pick_now_playing};
use crate::m3u::library_entry;
use crate::nowplaying::{current_from_activity, has_audio_history};
use crate::playlists::{PlaylistView, Store, StoreError};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NowPlayingView {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// False when the track is outside the curated library (an incoming folder, say): it cannot be added.
    pub addable: bool,
}

#[derive(Debug, Serialize)]
pub struct StateView {
    pub now_playing: Option<NowPlayingView>,
    pub playlists: Vec<PlaylistView>,
}

#[derive(Debug, thiserror::Error)]
pub enum ToggleError {
    #[error("nothing is playing")]
    NothingPlaying,
    #[error("the playing track is not in the curated library, so it cannot be added to a playlist")]
    NotInLibrary,
    #[error("unknown playlist")]
    UnknownPlaylist,
    #[error("{0:#}")]
    Upstream(#[from] anyhow::Error),
}

impl From<StoreError> for ToggleError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Unknown => ToggleError::UnknownPlaylist,
            StoreError::Io(e) => ToggleError::Upstream(e),
        }
    }
}

/// A track that started longer ago than this without a stop entry is assumed to be a stale log line.
const MAX_PLAYING_AGE_SECS: i64 = 3 * 3600;

fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct App {
    cfg: Config,
    jf: Jellyfin,
    store: Store,
    /// Details of recently seen tracks, so polling the playing track costs one cheap request.
    items: Mutex<HashMap<String, PlayingItem>>,
}

impl App {
    pub fn new(cfg: Config, jf: Jellyfin) -> Arc<Self> {
        let store = Store::new(&cfg.playlist_dir);
        Arc::new(Self { cfg, jf, store, items: Mutex::new(HashMap::new()) })
    }

    pub async fn playlist_count(&self) -> Result<usize> {
        Ok(self.store.view(None).await?.len())
    }

    /// What is playing. The activity log is the primary source because Jellyfin's session record can be stale
    /// with some clients; sessions are the fallback when the log is unavailable or has no audio history.
    async fn now_playing(&self) -> Result<Option<PlayingItem>> {
        match self.jf.activity(100).await {
            Ok(entries) if has_audio_history(&entries) => {
                return match current_from_activity(&entries, epoch_now(), MAX_PLAYING_AGE_SECS) {
                    Some(id) => Ok(Some(self.item(&id).await?)),
                    None => Ok(None),
                };
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("activity log unavailable, falling back to sessions: {e:#}"),
        }
        let sessions = self.jf.sessions().await?;
        Ok(pick_now_playing(&sessions, &self.cfg.player_client))
    }

    async fn item(&self, id: &str) -> Result<PlayingItem> {
        if let Some(hit) = self.items.lock().await.get(id) {
            return Ok(hit.clone());
        }
        let fetched = self.jf.item(id).await?;
        let mut cache = self.items.lock().await;
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(id.to_string(), fetched.clone());
        Ok(fetched)
    }

    /// The playlist-file form of the track's path, if it is in the curated library.
    fn entry_of(&self, item: &PlayingItem) -> Option<String> {
        item.path.as_deref().and_then(|p| library_entry(p, &self.cfg.music_prefixes))
    }

    pub async fn state(&self) -> Result<StateView> {
        let np = self.now_playing().await?;
        let entry = np.as_ref().and_then(|n| self.entry_of(n));
        let playlists = self.store.view(entry.as_deref()).await?;
        let now_playing = np.map(|n| NowPlayingView {
            addable: entry.is_some(),
            id: n.id,
            name: n.name,
            artist: n.album_artist,
            album: n.album,
        });
        Ok(StateView { now_playing, playlists })
    }

    /// Add the playing track to the playlist file, or remove it if it is already there.
    pub async fn toggle(self: &Arc<Self>, playlist_id: &str) -> Result<StateView, ToggleError> {
        let track = self.now_playing().await?.ok_or(ToggleError::NothingPlaying)?;
        let entry = self.entry_of(&track).ok_or(ToggleError::NotInLibrary)?;
        self.store.toggle(playlist_id, &entry).await?;
        // Best effort: tell Jellyfin the file changed so Finamp and the others see the new playlist.
        let jf = self.jf.clone();
        let path = format!("{}/{}.m3u", self.cfg.jellyfin_playlist_dir.trim_end_matches('/'), playlist_id);
        tokio::spawn(async move {
            if let Err(e) = jf.library_updated(&path).await {
                tracing::warn!("could not tell Jellyfin that {path} changed: {e:#}");
            }
        });
        Ok(self.state().await?)
    }
}
