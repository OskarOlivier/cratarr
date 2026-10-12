//! In-memory membership index: which playlists contain which tracks. Reads are instant; Jellyfin is slow
//! (it costs about 1 ms per playlist entry per request), so the index is built in the background, persisted
//! to disk, and refreshed only for playlists whose cheap fingerprint changed.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistState {
    pub name: String,
    /// What Jellyfin's cheap listing said when we last read the entries (entry count + runtime).
    pub fingerprint: String,
    /// False until the entries have been read at least once.
    pub indexed: bool,
    /// Needs a re-read regardless of fingerprint (we changed it ourselves).
    pub dirty: bool,
    /// track id -> entry ids. An empty list means "member, but entry ids not known yet" (just added).
    pub tracks: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlaylistView {
    pub id: String,
    pub name: String,
    pub member: bool,
    /// False while the playlist has not been indexed yet; `member` is then meaningless.
    pub known: bool,
}

/// One row of Jellyfin's cheap playlist listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    playlists: BTreeMap<String, PlaylistState>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Align with Jellyfin's listing: add new playlists, rename, drop deleted ones. Returns the ids that
    /// must be (re-)read: never indexed, fingerprint changed, or marked dirty.
    pub fn reconcile(&mut self, listing: &[Listed]) -> Vec<String> {
        let ids: Vec<&str> = listing.iter().map(|l| l.id.as_str()).collect();
        self.playlists.retain(|id, _| ids.contains(&id.as_str()));
        let mut stale = Vec::new();
        for l in listing {
            let st = self.playlists.entry(l.id.clone()).or_default();
            st.name = l.name.clone();
            if !st.indexed || st.dirty || st.fingerprint != l.fingerprint {
                st.fingerprint = l.fingerprint.clone();
                stale.push(l.id.clone());
            }
        }
        stale
    }

    /// Store freshly read entries (`(track id, entry id)` pairs).
    pub fn set_entries(&mut self, id: &str, entries: impl IntoIterator<Item = (String, String)>) {
        let Some(st) = self.playlists.get_mut(id) else { return };
        let mut tracks: HashMap<String, Vec<String>> = HashMap::new();
        for (track_id, entry_id) in entries {
            tracks.entry(track_id).or_default().push(entry_id);
        }
        st.tracks = tracks;
        st.indexed = true;
        st.dirty = false;
    }

    /// We just added `track_id` ourselves: show it immediately; the entry id is learned on the next read.
    pub fn mark_added(&mut self, id: &str, track_id: &str) {
        if let Some(st) = self.playlists.get_mut(id) {
            st.tracks.entry(track_id.to_string()).or_default();
            st.dirty = true;
        }
    }

    pub fn mark_removed(&mut self, id: &str, track_id: &str) {
        if let Some(st) = self.playlists.get_mut(id) {
            st.tracks.remove(track_id);
            st.dirty = true;
        }
    }

    pub fn len(&self) -> usize {
        self.playlists.len()
    }

    /// Playlists not yet indexed (for the "still indexing" indicator).
    pub fn unindexed(&self) -> usize {
        self.playlists.values().filter(|p| !p.indexed).count()
    }

    /// Every playlist, name-sorted (case-insensitive), flagged by whether it contains `track_id`.
    pub fn view(&self, track_id: Option<&str>) -> Vec<PlaylistView> {
        let mut v: Vec<PlaylistView> = self
            .playlists
            .iter()
            .map(|(id, p)| PlaylistView {
                id: id.clone(),
                name: p.name.clone(),
                member: p.indexed && track_id.is_some_and(|t| p.tracks.contains_key(t)),
                known: p.indexed,
            })
            .collect();
        v.sort_by_key(|p| p.name.to_lowercase());
        v
    }

    pub fn entry_ids(&self, playlist_id: &str, track_id: &str) -> Vec<String> {
        self.playlists
            .get(playlist_id)
            .and_then(|p| p.tracks.get(track_id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn is_member(&self, playlist_id: &str, track_id: &str) -> bool {
        self.playlists.get(playlist_id).is_some_and(|p| p.indexed && p.tracks.contains_key(track_id))
    }

    pub fn is_indexed(&self, playlist_id: &str) -> Option<bool> {
        self.playlists.get(playlist_id).map(|p| p.indexed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str, name: &str, fp: &str) -> Listed {
        Listed { id: id.into(), name: name.into(), fingerprint: fp.into() }
    }

    fn sample() -> Index {
        let mut i = Index::new();
        i.reconcile(&[listed("p1", "Chill", "2:10"), listed("p2", "analogic", "2:20"), listed("p3", "Beatless", "0:0")]);
        i.set_entries("p1", [("t1".into(), "e1".into()), ("t2".into(), "e2".into())]);
        i.set_entries("p2", [("t2".into(), "e3".into()), ("t2".into(), "e4".into())]);
        i.set_entries("p3", []);
        i
    }

    #[test]
    fn membership_is_flagged_per_playlist() {
        let v = sample().view(Some("t2"));
        let members: Vec<&str> = v.iter().filter(|p| p.member).map(|p| p.name.as_str()).collect();
        assert_eq!(members, vec!["analogic", "Chill"]);
    }

    #[test]
    fn view_lists_all_playlists_sorted_ignoring_case() {
        let names: Vec<String> = sample().view(None).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["analogic", "Beatless", "Chill"]);
    }

    #[test]
    fn no_track_means_nothing_is_highlighted() {
        assert!(sample().view(None).iter().all(|p| !p.member));
    }

    #[test]
    fn duplicate_entries_are_all_returned_for_removal() {
        assert_eq!(sample().entry_ids("p2", "t2"), vec!["e3", "e4"]);
        assert!(sample().entry_ids("p2", "t1").is_empty());
        assert!(sample().entry_ids("nope", "t1").is_empty());
    }

    #[test]
    fn new_playlists_start_unindexed_and_unknown() {
        let mut i = Index::new();
        let stale = i.reconcile(&[listed("p1", "Chill", "2:10")]);
        assert_eq!(stale, vec!["p1"]);
        assert_eq!(i.unindexed(), 1);
        let v = i.view(Some("t1"));
        assert!(!v[0].known && !v[0].member);
    }

    #[test]
    fn unchanged_fingerprint_is_not_re_read() {
        let mut i = sample();
        assert!(i.reconcile(&[listed("p1", "Chill", "2:10"), listed("p2", "analogic", "2:20"), listed("p3", "Beatless", "0:0")]).is_empty());
    }

    #[test]
    fn changed_fingerprint_marks_only_that_playlist_stale() {
        let mut i = sample();
        let stale = i.reconcile(&[listed("p1", "Chill", "3:15"), listed("p2", "analogic", "2:20"), listed("p3", "Beatless", "0:0")]);
        assert_eq!(stale, vec!["p1"]);
    }

    #[test]
    fn deleted_playlists_are_dropped_and_renames_applied() {
        let mut i = sample();
        i.reconcile(&[listed("p1", "Chilled", "2:10")]);
        assert_eq!(i.len(), 1);
        assert_eq!(i.view(None)[0].name, "Chilled");
    }

    #[test]
    fn our_own_add_shows_instantly_and_is_re_read_later() {
        let mut i = sample();
        i.mark_added("p1", "t9");
        assert!(i.is_member("p1", "t9"));
        assert!(i.entry_ids("p1", "t9").is_empty(), "entry id is not known yet");
        // dirty playlists are re-read even though Jellyfin's fingerprint has not changed
        let stale = i.reconcile(&[listed("p1", "Chill", "2:10"), listed("p2", "analogic", "2:20"), listed("p3", "Beatless", "0:0")]);
        assert_eq!(stale, vec!["p1"]);
        i.set_entries("p1", [("t9".into(), "e9".into())]);
        assert_eq!(i.entry_ids("p1", "t9"), vec!["e9"]);
    }

    #[test]
    fn our_own_removal_shows_instantly() {
        let mut i = sample();
        i.mark_removed("p1", "t1");
        assert!(!i.is_member("p1", "t1"));
    }

    #[test]
    fn index_survives_a_json_round_trip() {
        let j = sample().to_json().unwrap();
        let back = Index::from_json(&j).unwrap();
        assert_eq!(back.view(Some("t2")), sample().view(Some("t2")));
        assert_eq!(back.unindexed(), 0);
    }
}
