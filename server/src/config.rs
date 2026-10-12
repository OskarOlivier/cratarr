//! Configuration, read from environment variables so the container needs no config file.

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on, e.g. `0.0.0.0:8080`.
    pub bind: String,
    /// Jellyfin base URL without a trailing slash.
    pub jellyfin_url: String,
    pub api_key: String,
    /// Jellyfin user whose playlists are managed (dashed or undashed GUID).
    pub user_id: String,
    /// Path prefixes (as Jellyfin reports them) whose tracks may never be added to playlists.
    pub incoming_prefixes: Vec<String>,
    /// Preferred player client name when several sessions are playing audio.
    pub player_client: String,
    /// Directory for the persisted index cache.
    pub data_dir: String,
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
        Ok(Self {
            bind: lookup("CRATARR_BIND").unwrap_or_else(|| "0.0.0.0:8080".into()),
            jellyfin_url,
            api_key: need("JELLYFIN_API_KEY")?,
            user_id: need("JELLYFIN_USER_ID")?,
            incoming_prefixes: lookup("CRATARR_INCOMING_PREFIXES")
                .map(|v| {
                    v.split(',')
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            player_client: lookup("CRATARR_PLAYER_CLIENT").unwrap_or_else(|| "Finamp".into()),
            data_dir: lookup("CRATARR_DATA_DIR").unwrap_or_else(|| "/data".into()),
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

    #[test]
    fn minimal_config_gets_defaults() {
        let c = cfg(&[("JELLYFIN_URL", "http://jf:8096/"), ("JELLYFIN_API_KEY", "k"), ("JELLYFIN_USER_ID", "u")]).unwrap();
        assert_eq!(c.jellyfin_url, "http://jf:8096");
        assert_eq!(c.bind, "0.0.0.0:8080");
        assert_eq!(c.player_client, "Finamp");
        assert_eq!(c.data_dir, "/data");
        assert!(c.incoming_prefixes.is_empty());
    }

    #[test]
    fn missing_required_value_is_an_error_naming_it() {
        let e = cfg(&[("JELLYFIN_URL", "http://jf:8096"), ("JELLYFIN_USER_ID", "u")]).unwrap_err();
        assert!(e.to_string().contains("JELLYFIN_API_KEY"));
    }

    #[test]
    fn url_without_scheme_is_rejected() {
        assert!(cfg(&[("JELLYFIN_URL", "jf:8096"), ("JELLYFIN_API_KEY", "k"), ("JELLYFIN_USER_ID", "u")]).is_err());
    }

    #[test]
    fn incoming_prefixes_are_split_and_trimmed() {
        let c = cfg(&[
            ("JELLYFIN_URL", "http://jf"),
            ("JELLYFIN_API_KEY", "k"),
            ("JELLYFIN_USER_ID", "u"),
            ("CRATARR_INCOMING_PREFIXES", " /data/media/incoming , /other/ ,"),
        ])
        .unwrap();
        assert_eq!(c.incoming_prefixes, vec!["/data/media/incoming", "/other/"]);
    }
}
