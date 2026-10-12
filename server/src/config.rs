//! Configuration, read from environment variables so the container needs no config file.

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on, e.g. `0.0.0.0:8080`.
    pub bind: String,
    /// Jellyfin base URL without a trailing slash.
    pub jellyfin_url: String,
    pub api_key: String,
    /// Jellyfin user whose "now playing" is followed (dashed or undashed GUID).
    pub user_id: String,
    /// Preferred player client name when several sessions are playing audio (fallback source only).
    pub player_client: String,
    /// Folder holding the `.m3u` playlists, as this process sees it.
    pub playlist_dir: String,
    /// Path prefixes, as Jellyfin reports track paths, that make up the curated library. Everything under one of
    /// them maps to a playlist entry; tracks outside them (an incoming folder, say) can never be added.
    pub music_prefixes: Vec<String>,
}

fn split_list(v: &str) -> Vec<String> {
    v.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Testable core: `lookup` stands in for the environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let need = |key: &str| -> Result<String> {
            lookup(key)
                .filter(|v| !v.trim().is_empty())
                .with_context(|| format!("environment variable {key} is required"))
        };
        let jellyfin_url = need("JELLYFIN_URL")?.trim_end_matches('/').to_string();
        if !jellyfin_url.starts_with("http://") && !jellyfin_url.starts_with("https://") {
            bail!("JELLYFIN_URL must start with http:// or https://");
        }
        let music_prefixes = lookup("CRATARR_MUSIC_PREFIXES")
            .map(|v| split_list(&v))
            .unwrap_or_else(|| vec!["/data/media/Music".into(), "/Volume1/media/Music".into()]);
        if music_prefixes.is_empty() {
            bail!("CRATARR_MUSIC_PREFIXES must list at least one path");
        }
        Ok(Self {
            bind: lookup("CRATARR_BIND").unwrap_or_else(|| "0.0.0.0:8080".into()),
            jellyfin_url,
            api_key: need("JELLYFIN_API_KEY")?,
            user_id: need("JELLYFIN_USER_ID")?,
            player_client: lookup("CRATARR_PLAYER_CLIENT").unwrap_or_else(|| "Finamp".into()),
            playlist_dir: need("CRATARR_PLAYLIST_DIR")?,
            music_prefixes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_lookup(|k| m.get(k).cloned())
    }

    const BASE: [(&str, &str); 4] = [
        ("JELLYFIN_URL", "http://jf:8096/"),
        ("JELLYFIN_API_KEY", "k"),
        ("JELLYFIN_USER_ID", "u"),
        ("CRATARR_PLAYLIST_DIR", "/music"),
    ];

    #[test]
    fn minimal_config_gets_defaults() {
        let c = cfg(&BASE).unwrap();
        assert_eq!(c.jellyfin_url, "http://jf:8096");
        assert_eq!(c.bind, "0.0.0.0:8080");
        assert_eq!(c.player_client, "Finamp");
        assert_eq!(c.playlist_dir, "/music");
        assert_eq!(c.music_prefixes, vec!["/data/media/Music", "/Volume1/media/Music"]);
    }

    #[test]
    fn each_required_value_is_named_when_missing() {
        for missing in ["JELLYFIN_URL", "JELLYFIN_API_KEY", "JELLYFIN_USER_ID", "CRATARR_PLAYLIST_DIR"] {
            let pairs: Vec<_> = BASE.iter().filter(|(k, _)| *k != missing).cloned().collect();
            let e = cfg(&pairs).unwrap_err();
            assert!(e.to_string().contains(missing), "{e}");
        }
    }

    #[test]
    fn url_without_scheme_is_rejected() {
        let mut pairs = BASE.to_vec();
        pairs[0] = ("JELLYFIN_URL", "jf:8096");
        assert!(cfg(&pairs).is_err());
    }

    #[test]
    fn music_prefixes_are_split_and_trimmed() {
        let mut pairs = BASE.to_vec();
        pairs.push(("CRATARR_MUSIC_PREFIXES", " /a/Music , /b/Music/ ,"));
        assert_eq!(cfg(&pairs).unwrap().music_prefixes, vec!["/a/Music", "/b/Music/"]);
    }

    #[test]
    fn an_empty_prefix_list_is_rejected() {
        let mut pairs = BASE.to_vec();
        pairs.push(("CRATARR_MUSIC_PREFIXES", " , "));
        assert!(cfg(&pairs).is_err());
    }
}
