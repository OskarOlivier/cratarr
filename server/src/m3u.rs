//! `.m3u` text handling. Playlists are plain files with one library-relative path per line
//! (`Folder/File.mp3`); `#` lines are comments. Edits touch only the lines they must, so the rest of the file
//! (comments, odd entries, line endings, a UTF-8 BOM) is preserved byte for byte.

/// A track's library-relative path, as it appears in playlists, from the path Jellyfin reports for it.
/// `None` when the track is not inside one of the library prefixes (for example an incoming/sandbox track).
pub fn library_entry(path: &str, prefixes: &[String]) -> Option<String> {
    let p = path.replace('\\', "/");
    prefixes.iter().find_map(|pre| {
        let pre = pre.replace('\\', "/");
        let pre = pre.trim_end_matches('/');
        if pre.is_empty() {
            return None;
        }
        let rel = p.strip_prefix(pre)?.strip_prefix('/')?;
        (!rel.is_empty()).then(|| rel.to_string())
    })
}

/// The comparable form of a playlist line, or `None` for blanks and comments.
fn entry_of(line: &str) -> Option<String> {
    let t = line.trim_start_matches('\u{feff}').trim();
    (!t.is_empty() && !t.starts_with('#')).then(|| t.replace('\\', "/"))
}

pub fn entries(content: &str) -> impl Iterator<Item = String> + '_ {
    content.lines().filter_map(entry_of)
}

pub fn contains(content: &str, entry: &str) -> bool {
    entries(content).any(|e| e == entry)
}

/// Append `entry`, keeping the file's line-ending style.
pub fn with_added(content: &str, entry: &str) -> String {
    let eol = if content.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(content.len() + entry.len() + 2);
    out.push_str(content);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(eol);
    }
    out.push_str(entry);
    out.push_str(eol);
    out
}

/// Remove every line equal to `entry`; returns the new text and how many lines went.
pub fn with_removed(content: &str, entry: &str) -> (String, usize) {
    let mut removed = 0;
    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        if entry_of(line).as_deref() == Some(entry) {
            removed += 1;
        } else {
            out.push_str(line);
        }
    }
    (out, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn library_entry_strips_the_first_matching_prefix() {
        let pre = p(&["/data/media/Music", "/Volume1/media/Music"]);
        assert_eq!(
            library_entry("/data/media/Music/Artist - Album (2020)/01 Artist - Song.mp3", &pre).as_deref(),
            Some("Artist - Album (2020)/01 Artist - Song.mp3")
        );
        assert_eq!(
            library_entry("/Volume1/media/Music/A/B.flac", &pre).as_deref(),
            Some("A/B.flac")
        );
    }

    #[test]
    fn tracks_outside_the_library_have_no_entry() {
        let pre = p(&["/data/media/Music"]);
        assert_eq!(library_entry("/data/media/lidarr-music/A/B.flac", &pre), None);
        assert_eq!(library_entry("/data/media/Music-old/A/B.flac", &pre), None);
        assert_eq!(library_entry("/data/media/Music", &pre), None);
        assert_eq!(library_entry("/data/media/Music/", &pre), None);
        assert_eq!(library_entry("/x/y.flac", &[]), None);
        assert_eq!(library_entry("/x/y.flac", &p(&["", "/"])), None);
    }

    #[test]
    fn backslashes_and_trailing_slashes_are_tolerated() {
        assert_eq!(library_entry(r"\data\media\Music\A\B.mp3", &p(&["/data/media/Music/"])).as_deref(), Some("A/B.mp3"));
    }

    #[test]
    fn entries_skip_comments_and_blanks_and_a_bom() {
        let c = "\u{feff}#EXTM3U\n\nA/1.mp3\r\n  B/2.mp3  \n# note\nC\\3.mp3\n";
        assert_eq!(entries(c).collect::<Vec<_>>(), vec!["A/1.mp3", "B/2.mp3", "C/3.mp3"]);
        assert!(contains(c, "B/2.mp3"));
        assert!(!contains(c, "note"));
    }

    #[test]
    fn adding_appends_with_the_files_own_line_endings() {
        assert_eq!(with_added("A/1.mp3\n", "B/2.mp3"), "A/1.mp3\nB/2.mp3\n");
        assert_eq!(with_added("A/1.mp3\r\n", "B/2.mp3"), "A/1.mp3\r\nB/2.mp3\r\n");
        assert_eq!(with_added("A/1.mp3", "B/2.mp3"), "A/1.mp3\nB/2.mp3\n", "a missing final newline is repaired");
        assert_eq!(with_added("", "B/2.mp3"), "B/2.mp3\n");
    }

    #[test]
    fn removing_drops_every_copy_and_nothing_else() {
        let c = "#c\nA/1.mp3\nB/2.mp3\r\nA/1.mp3\nC/3.mp3";
        let (out, n) = with_removed(c, "A/1.mp3");
        assert_eq!(n, 2);
        assert_eq!(out, "#c\nB/2.mp3\r\nC/3.mp3");
    }

    #[test]
    fn removing_something_absent_changes_nothing() {
        let c = "A/1.mp3\nB/2.mp3\n";
        assert_eq!(with_removed(c, "X/9.mp3"), (c.to_string(), 0));
    }

    #[test]
    fn removal_matches_across_backslash_and_whitespace_differences() {
        let (out, n) = with_removed("  A\\1.mp3  \nB/2.mp3\n", "A/1.mp3");
        assert_eq!((out.as_str(), n), ("B/2.mp3\n", 1));
    }

    #[test]
    fn odd_existing_lines_survive_untouched() {
        let c = "Owiny Sigoma Band - Power Punch!!! (2013)/\nA/1.mp3\n";
        let (out, _) = with_removed(c, "A/1.mp3");
        assert_eq!(out, "Owiny Sigoma Band - Power Punch!!! (2013)/\n");
    }
}
