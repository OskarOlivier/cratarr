//! Application logic: keeps the index in step with Jellyfin (gently, in the background) and decides what a
//! toggle does. Taps update the index immediately; the slow re-read of the playlist happens afterwards.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::guard::is_incoming;
use crate::index::{Index, Listed, PlaylistView};
use crate::jellyfin::{Jellyfin, PlayingItem, pick_now_playing};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NowPlayingView {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// True when the track is in an incoming path and therefore cannot be added to playlists.
    pub incoming: bool,
}

#[derive(Debug, Serialize)]
pub struct StateView {
    pub now_playing: Option<NowPlayingView>,
    pub playlists: Vec<PlaylistView>,
    /// Playlists still waiting for their first read (the index is built in the background).
    pub unindexed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ToggleError {
    #[error("nothing is playing")]
    NothingPlaying,
    #[error("the playing track is still in the incoming library and cannot be added to a playlist")]
    Incoming,
    #[error("unknown playlist")]
    UnknownPlaylist,
    #[error("this playlist has not been indexed yet, try again in a moment")]
    NotIndexedYet,
    #[error("Jellyfin error: {0}")]
    Upstream(#[from] anyhow::Error),
}

pub struct App {
    cfg: Config,
    jf: Jellyfin,
    index: RwLock<Index>,
    /// One Jellyfin read at a time: its playlist reads are heavy and we never want to hammer it.
    reading: Mutex<()>,
}

impl App {
    pub fn new(cfg: Config, jf: Jellyfin) -> Arc<Self> {
        Arc::new(Self { cfg, jf, index: RwLock::new(Index::new()), reading: Mutex::new(()) })
    }

    fn cache_path(&self) -> PathBuf {
        PathBuf::from(&self.cfg.data_dir).join("index.json")
    }

    /// Load the persisted index so a restart is instant. A missing or unreadable cache is not an error.
    pub async fn load_cache(&self) {
        match tokio::fs::read_to_string(self.cache_path()).await {
            Ok(s) => match Index::from_json(&s) {
                Ok(idx) => {
                    tracing::info!("loaded cached index with {} playlists", idx.len());
                    *self.index.write().await = idx;
                }
                Err(e) => tracing::warn!("ignoring unreadable index cache: {e}"),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("cannot read index cache: {e}"),
        }
    }

    async fn save_cache(&self) {
        let json = match self.index.read().await.to_json() {
            Ok(j) => j,
            Err(e) => return tracing::warn!("cannot serialise index: {e}"),
        };
        let path = self.cache_path();
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = async {
            if let Some(dir) = path.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            tokio::fs::write(&tmp, json).await?;
            tokio::fs::rename(&tmp, &path).await
        }
        .await
        {
            tracing::warn!("cannot write index cache: {e}");
        }
    }

    /// Bring the index up to date: cheap listing first, then read only the playlists that are new, changed or
    /// dirty, one at a time. Failures are logged and retried on the next pass.
    pub async fn sync(&self) -> Result<()> {
        let listing: Vec<Listed> = self
            .jf
            .playlists()
            .await?
            .into_iter()
            .map(|p| Listed { fingerprint: p.fingerprint(), id: p.id, name: p.name })
            .collect();
        let stale = self.index.write().await.reconcile(&listing);
        self.save_cache().await;
        if !stale.is_empty() {
            tracing::info!("{} playlist(s) to read", stale.len());
        }
        for id in stale {
            if let Err(e) = self.read_playlist(&id).await {
                tracing::warn!("reading playlist {id} failed: {e:#}");
            }
        }
        Ok(())
    }

    async fn read_playlist(&self, id: &str) -> Result<()> {
        let _one_at_a_time = self.reading.lock().await;
        let entries = self.jf.entries(id).await?;
        self.index
            .write()
            .await
            .set_entries(id, entries.into_iter().map(|e| (e.id, e.playlist_item_id)));
        self.save_cache().await;
        Ok(())
    }

    async fn now_playing(&self) -> Result<Option<PlayingItem>> {
        let sessions = self.jf.sessions().await?;
        Ok(pick_now_playing(&sessions, &self.cfg.player_client))
    }

    pub async fn state(&self) -> Result<StateView> {
        let np = self.now_playing().await?;
        let idx = self.index.read().await;
        let playlists = idx.view(np.as_ref().map(|n| n.id.as_str()));
        let unindexed = idx.unindexed();
        drop(idx);
        let now_playing = np.map(|n| NowPlayingView {
            incoming: n.path.as_deref().is_some_and(|p| is_incoming(p, &self.cfg.incoming_prefixes)),
            id: n.id,
            name: n.name,
            artist: n.album_artist,
            album: n.album,
        });
        Ok(StateView { now_playing, playlists, unindexed })
    }

    /// Add the playing track to the playlist, or remove it if it is already there. The index changes at once;
    /// the playlist is re-read in the background. Adding an incoming track is refused; removing one is allowed.
    pub async fn toggle(self: &Arc<Self>, playlist_id: &str) -> Result<StateView, ToggleError> {
        let track = self.now_playing().await?.ok_or(ToggleError::NothingPlaying)?;
        let member = {
            let idx = self.index.read().await;
            match idx.is_indexed(playlist_id) {
                None => return Err(ToggleError::UnknownPlaylist),
                Some(false) => return Err(ToggleError::NotIndexedYet),
                Some(true) => idx.is_member(playlist_id, &track.id),
            }
        };
        if !member {
            if track.path.as_deref().is_some_and(|p| is_incoming(p, &self.cfg.incoming_prefixes)) {
                return Err(ToggleError::Incoming);
            }
            self.jf.add(playlist_id, &track.id).await?;
            self.index.write().await.mark_added(playlist_id, &track.id);
        } else {
            let mut entries = self.index.read().await.entry_ids(playlist_id, &track.id);
            if entries.is_empty() {
                // Added by us a moment ago: the entry id is not known until the playlist is re-read.
                self.read_playlist(playlist_id).await?;
                entries = self.index.read().await.entry_ids(playlist_id, &track.id);
            }
            if !entries.is_empty() {
                self.jf.remove(playlist_id, &entries).await?;
            }
            self.index.write().await.mark_removed(playlist_id, &track.id);
        }
        self.save_cache().await;
        let app = Arc::clone(self);
        let id = playlist_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = app.read_playlist(&id).await {
                tracing::warn!("background re-read of {id} failed: {e:#}");
            }
        });
        Ok(self.state().await?)
    }

    pub async fn playlist_count(&self) -> usize {
        self.index.read().await.len()
    }
}
