//! Thin Jellyfin API client. Only the parts Cratarr needs; unknown fields are ignored so harmless
//! additions by Jellyfin never break us, while the fields we rely on are required and fail loudly.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsPage<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub child_count: Option<u64>,
    pub run_time_ticks: Option<u64>,
}

impl Playlist {
    /// Cheap change signal from the listing: reading a playlist's entries is slow, the listing is not.
    pub fn fingerprint(&self) -> String {
        format!("{}:{}", self.child_count.unwrap_or(0), self.run_time_ticks.unwrap_or(0))
    }
}

/// One row of a playlist: `id` is the track, `playlist_item_id` identifies this entry for removal.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Entry {
    pub id: String,
    pub playlist_item_id: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicInfo {
    pub version: String,
}

/// Choose the audio item to act on: audio sessions only, preferring the configured client.
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

    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        let page: ItemsPage<Playlist> = self
            .get(&format!("/Users/{}/Items?IncludeItemTypes=Playlist&Recursive=true&Limit=1000", self.user_id))
            .await?;
        Ok(page.items)
    }

    /// Slow for large playlists (measured: ~1 ms per entry just to resolve the playlist, plus ~8 ms per entry
    /// returned; 5,000 entries took about 50 s), so it has its own generous timeout.
    pub async fn entries(&self, playlist_id: &str) -> Result<Vec<Entry>> {
        let path = format!(
            "/Playlists/{playlist_id}/Items?UserId={}&Limit=100000&EnableImages=false&EnableUserData=false&ImageTypeLimit=0",
            self.user_id
        );
        let resp = self
            .http
            .get(format!("{}{}", self.base, path))
            .timeout(std::time::Duration::from_secs(600))
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        let status = resp.status();
        if !status.is_success() {
            bail!("GET /Playlists/{playlist_id}/Items returned {status}");
        }
        let page: ItemsPage<Entry> = resp
            .json()
            .await
            .context("playlist entries response did not have the expected shape")?;
        Ok(page.items)
    }

    pub async fn sessions(&self) -> Result<Vec<Session>> {
        self.get("/Sessions").await
    }

    pub async fn add(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let url = format!("{}/Playlists/{playlist_id}/Items?ids={track_id}&userId={}", self.base, self.user_id);
        let status = self.http.post(&url).send().await.context("add to playlist")?.status();
        if !status.is_success() {
            bail!("adding to playlist returned {status}");
        }
        Ok(())
    }

    pub async fn remove(&self, playlist_id: &str, entry_ids: &[String]) -> Result<()> {
        let url = format!("{}/Playlists/{playlist_id}/Items?entryIds={}", self.base, entry_ids.join(","));
        let status = self.http.delete(&url).send().await.context("remove from playlist")?.status();
        if !status.is_success() {
            bail!("removing from playlist returned {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLISTS: &str = include_str!("../tests/fixtures/playlists.json");
    const ENTRIES: &str = include_str!("../tests/fixtures/playlist_entries.json");
    const SESSIONS: &str = include_str!("../tests/fixtures/sessions.json");

    #[test]
    fn playlists_fixture_parses() {
        let page: ItemsPage<Playlist> = serde_json::from_str(PLAYLISTS).unwrap();
        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(|p| p.id.len() == 32 && !p.name.is_empty()));
    }

    #[test]
    fn playlist_fingerprint_comes_from_count_and_runtime() {
        let a = Playlist { id: "a".into(), name: "A".into(), child_count: Some(3), run_time_ticks: Some(100) };
        let b = Playlist { child_count: Some(4), ..a.clone() };
        assert_eq!(a.fingerprint(), "3:100");
        assert_ne!(a.fingerprint(), b.fingerprint());
        let bare: Playlist = serde_json::from_str(r#"{"Id":"x","Name":"X"}"#).unwrap();
        assert_eq!(bare.fingerprint(), "0:0");
    }

    #[test]
    fn entries_fixture_parses_with_entry_ids() {
        let page: ItemsPage<Entry> = serde_json::from_str(ENTRIES).unwrap();
        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(|e| !e.playlist_item_id.is_empty()));
    }

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
    fn missing_required_field_fails_loudly() {
        let bad = r#"{"Items":[{"Name":"No id here"}]}"#;
        assert!(serde_json::from_str::<ItemsPage<Playlist>>(bad).is_err());
    }
}
