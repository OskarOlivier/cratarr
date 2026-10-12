//! Thin Jellyfin API client. Jellyfin supplies what is playing and is told when a playlist file changed; the
//! playlists themselves are `.m3u` files (see `playlists.rs`). Unknown fields are ignored so harmless additions
//! by Jellyfin never break us, while the fields we rely on are required and fail loudly.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsPage<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Session {
    pub client: Option<String>,
    pub now_playing_item: Option<PlayingItem>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct PlayingItem {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub media_type: Option<String>,
}

/// One row of Jellyfin's activity log; only audio playback start/stop entries matter to us.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ActivityEntry {
    pub date: String,
    #[serde(rename = "Type")]
    pub kind: String,
    pub item_id: Option<String>,
}

/// A playlist as listed by Jellyfin (the cheap listing; reading a playlist's entries is the slow part).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListedPlaylist {
    pub id: String,
    pub name: String,
}

pub fn pick_playlist_id(items: &[ListedPlaylist], name: &str) -> Option<String> {
    items.iter().find(|p| p.name == name).map(|p| p.id.clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicInfo {
    pub version: String,
}

/// Fallback source for "what is playing": audio sessions only, preferring the configured client.
pub fn pick_now_playing(sessions: &[Session], preferred_client: &str) -> Option<PlayingItem> {
    let audio = |s: &&Session| {
        s.now_playing_item
            .as_ref()
            .is_some_and(|i| i.media_type.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("Audio")))
    };
    let preferred = sessions
        .iter()
        .filter(audio)
        .find(|s| s.client.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(preferred_client)));
    preferred
        .or_else(|| sessions.iter().find(audio))
        .and_then(|s| s.now_playing_item.clone())
}

#[derive(Clone)]
pub struct Jellyfin {
    http: reqwest::Client,
    base: String,
    user_id: String,
}

impl Jellyfin {
    pub fn new(base: &str, api_key: &str, user_id: &str) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut token = reqwest::header::HeaderValue::from_str(api_key).context("API key has invalid characters")?;
        token.set_sensitive(true);
        headers.insert("X-Emby-Token", token);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        Ok(Self { http, base: base.trim_end_matches('/').to_string(), user_id: user_id.to_string() })
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.get(&url).send().await.with_context(|| format!("GET {path}"))?;
        let status = resp.status();
        if !status.is_success() {
            bail!("GET {path} returned {status}");
        }
        resp.json::<T>().await.with_context(|| format!("GET {path}: response did not have the expected shape"))
    }

    /// Unauthenticated; also proves the server is reachable.
    pub async fn public_info(&self) -> Result<PublicInfo> {
        self.get("/System/Info/Public").await
    }

    pub async fn sessions(&self) -> Result<Vec<Session>> {
        self.get("/Sessions").await
    }

    /// Newest entries first.
    pub async fn activity(&self, limit: u32) -> Result<Vec<ActivityEntry>> {
        let page: ItemsPage<ActivityEntry> = self.get(&format!("/System/ActivityLog/Entries?limit={limit}")).await?;
        Ok(page.items)
    }

    /// One item with the fields shown on the page.
    pub async fn item(&self, id: &str) -> Result<PlayingItem> {
        self.get(&format!("/Users/{}/Items/{id}", self.user_id)).await
    }

    /// Ask Jellyfin to re-read one imported playlist after its `.m3u` file changed. `name` is the file name
    /// without `.m3u`, which is also the playlist item's name.
    ///
    /// Verified against a real server (10.11): a "file changed" notice (`/Library/Media/Updated`) does nothing,
    /// the `ValidationOnly` and `Default` refresh modes do nothing, plain `FullRefresh` picks up added entries
    /// but keeps removed ones, and `FullRefresh` with `ReplaceAllMetadata=true` handles both (the entry count
    /// followed the file within seconds). Parameters go in the query string, as the API expects.
    pub async fn refresh_playlist(&self, name: &str) -> Result<()> {
        let listing: ItemsPage<ListedPlaylist> = self
            .get(&format!("/Users/{}/Items?IncludeItemTypes=Playlist&Recursive=true&Limit=1000", self.user_id))
            .await?;
        let Some(id) = pick_playlist_id(&listing.items, name) else {
            bail!("Jellyfin has no playlist named {name:?}");
        };
        let url = format!(
            "{}/Items/{id}/Refresh?Recursive=false&ImageRefreshMode=None&MetadataRefreshMode=FullRefresh&ReplaceAllMetadata=true",
            self.base
        );
        let status = self.http.post(&url).send().await.context("POST /Items/{id}/Refresh")?.status();
        if !status.is_success() {
            bail!("refreshing playlist {name:?} returned {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSIONS: &str = include_str!("../tests/fixtures/sessions.json");

    #[test]
    fn sessions_fixture_parses() {
        let s: Vec<Session> = serde_json::from_str(SESSIONS).unwrap();
        assert!(!s.is_empty());
    }

    #[test]
    fn now_playing_comes_from_the_audio_session() {
        let s: Vec<Session> = serde_json::from_str(SESSIONS).unwrap();
        let np = pick_now_playing(&s, "Finamp").expect("fixture has an audio session");
        assert_eq!(np.media_type.as_deref(), Some("Audio"));
        assert!(np.path.is_some());
    }

    fn sess(client: &str, id: &str, media: &str) -> Session {
        Session {
            client: Some(client.into()),
            now_playing_item: Some(PlayingItem {
                id: id.into(),
                name: id.into(),
                path: None,
                album: None,
                album_artist: None,
                media_type: Some(media.into()),
            }),
        }
    }

    #[test]
    fn preferred_client_wins_among_audio_sessions() {
        let s = vec![sess("Web", "a", "Audio"), sess("finamp", "b", "Audio")];
        assert_eq!(pick_now_playing(&s, "Finamp").unwrap().id, "b");
    }

    #[test]
    fn video_sessions_are_ignored() {
        let s = vec![sess("Finamp", "v", "Video"), sess("Web", "a", "Audio")];
        assert_eq!(pick_now_playing(&s, "Finamp").unwrap().id, "a");
    }

    #[test]
    fn nothing_playing_gives_none() {
        let s = vec![Session { client: Some("Finamp".into()), now_playing_item: None }];
        assert!(pick_now_playing(&s, "Finamp").is_none());
        assert!(pick_now_playing(&[], "Finamp").is_none());
    }

    #[test]
    fn a_playlist_is_found_by_exact_name() {
        let items: Vec<ListedPlaylist> =
            serde_json::from_str(r#"[{"Id":"a","Name":"Chill"},{"Id":"b","Name":"Chill Out"}]"#).unwrap();
        assert_eq!(pick_playlist_id(&items, "Chill").as_deref(), Some("a"));
        assert_eq!(pick_playlist_id(&items, "chill"), None);
        assert_eq!(pick_playlist_id(&[], "Chill"), None);
    }

    #[test]
    fn a_playing_item_without_an_id_fails_loudly() {
        let bad = r#"[{"Client":"Finamp","NowPlayingItem":{"Name":"No id here"}}]"#;
        assert!(serde_json::from_str::<Vec<Session>>(bad).is_err());
    }
}
