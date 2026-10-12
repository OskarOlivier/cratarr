//! The incoming-path rule: tracks that live under an "incoming" prefix are not curated yet, so they must
//! never be added to a playlist.

/// True when `path` is inside one of the `prefixes`. Matching is case-insensitive, treats `\` as `/`, and
/// only matches whole path segments (`/media/in` does not cover `/media/incoming`).
pub fn is_incoming(path: &str, prefixes: &[String]) -> bool {
    let path = normalise(path);
    prefixes.iter().any(|p| {
        let p = normalise(p);
        let p = p.trim_end_matches('/');
        !p.is_empty() && (path == p || path.starts_with(&format!("{p}/")))
    })
}

fn normalise(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn file_inside_prefix_is_incoming() {
        assert!(is_incoming("/data/media/incoming/Artist/Album (2020)/01 Song.flac", &p(&["/data/media/incoming"])));
    }

    #[test]
    fn curated_library_path_is_not_incoming() {
        assert!(!is_incoming("/data/media/Music/Artist - Album (2020)/01 Song.flac", &p(&["/data/media/incoming"])));
    }

    #[test]
    fn only_whole_segments_match() {
        assert!(!is_incoming("/data/media/incoming-old/a.flac", &p(&["/data/media/incoming"])));
        assert!(!is_incoming("/data/media/inc/a.flac", &p(&["/data/media/incoming"])));
    }

    #[test]
    fn case_slashes_and_trailing_slash_are_ignored() {
        assert!(is_incoming(r"\DATA\Media\Incoming\a.flac", &p(&["/data/media/incoming/"])));
    }

    #[test]
    fn no_prefixes_means_nothing_is_incoming() {
        assert!(!is_incoming("/anything/at/all.flac", &[]));
    }

    #[test]
    fn empty_prefix_never_matches_everything() {
        assert!(!is_incoming("/anything.flac", &p(&["", "/"])));
    }
}
