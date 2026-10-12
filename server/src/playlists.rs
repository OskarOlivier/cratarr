//! The playlists: `.m3u` files in one folder. Reads are cheap (small text files, cached by modification stamp),
//! writes are careful: re-read right before the edit, back the file up, write a temporary file next to it and
//! rename it into place, so a crash or a concurrent edit can never leave a half-written playlist.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::m3u;

const BACKUP_DIR: &str = ".cratarr-backups";
const KEEP_BACKUPS: usize = 20;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlaylistView {
    pub id: String,
    pub name: String,
    pub member: bool,
    /// Number of entries (lines that are not blank or comments).
    pub count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unknown playlist")]
    Unknown,
    #[error(transparent)]
    Io(#[from] anyhow::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Toggled {
    pub added: bool,
}

struct Cached {
    stamp: (SystemTime, u64),
    entries: HashSet<String>,
    count: usize,
}

impl Cached {
    fn parse(stamp: (SystemTime, u64), text: &str) -> Self {
        Self { stamp, entries: m3u::entries(text).collect(), count: m3u::entries(text).count() }
    }
}

pub struct Store {
    dir: PathBuf,
    cache: RwLock<HashMap<String, Cached>>,
    write: Mutex<()>,
}

/// A playlist id is its file name without `.m3u`; it must never be able to point outside the folder.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 200 && !id.starts_with('.') && !id.contains(['/', '\\', '\0'])
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), cache: RwLock::new(HashMap::new()), write: Mutex::new(()) }
    }

    fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.m3u"))
    }

    /// `(id, path, stamp)` for every visible `.m3u` file directly in the folder.
    async fn list(&self) -> Result<Vec<(String, PathBuf, (SystemTime, u64))>> {
        let mut rd = tokio::fs::read_dir(&self.dir)
            .await
            .with_context(|| format!("cannot read the playlist folder {}", self.dir.display()))?;
        let mut out = Vec::new();
        while let Some(e) = rd.next_entry().await? {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !name.to_lowercase().ends_with(".m3u") {
                continue;
            }
            let Ok(meta) = tokio::fs::metadata(e.path()).await else { continue };
            if !meta.is_file() {
                continue;
            }
            let id = name[..name.len() - 4].to_string();
            out.push((id, e.path(), (meta.modified().unwrap_or(UNIX_EPOCH), meta.len())));
        }
        Ok(out)
    }

    /// Every playlist, name-sorted (case-insensitive), flagged by whether it contains `entry`.
    pub async fn view(&self, entry: Option<&str>) -> Result<Vec<PlaylistView>> {
        let files = self.list().await?;
        let mut cache = self.cache.write().await;
        cache.retain(|id, _| files.iter().any(|f| &f.0 == id));
        let mut out = Vec::with_capacity(files.len());
        for (id, path, stamp) in files {
            if !cache.get(&id).is_some_and(|c| c.stamp == stamp) {
                match tokio::fs::read(&path).await {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        cache.insert(id.clone(), Cached::parse(stamp, &text));
                    }
                    Err(e) => {
                        tracing::warn!("cannot read {}: {e}", path.display());
                        continue;
                    }
                }
            }
            let c = &cache[&id];
            let member = entry.is_some_and(|t| c.entries.contains(t));
            out.push(PlaylistView { name: id.clone(), member, count: c.count, id });
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    /// Add `entry` to the playlist, or remove it if it is already there.
    pub async fn toggle(&self, id: &str, entry: &str) -> Result<Toggled, StoreError> {
        if !valid_id(id) {
            return Err(StoreError::Unknown);
        }
        let path = self.path_of(id);
        let _one_writer = self.write.lock().await;
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) if m.is_file() => m,
            _ => return Err(StoreError::Unknown),
        };
        let bytes = tokio::fs::read(&path).await.with_context(|| format!("reading {id}.m3u"))?;
        let text = String::from_utf8(bytes).map_err(|_| anyhow!("{id}.m3u is not valid UTF-8, refusing to edit it"))?;
        let (new_text, added) = if m3u::contains(&text, entry) {
            (m3u::with_removed(&text, entry).0, false)
        } else {
            (m3u::with_added(&text, entry), true)
        };
        self.backup(id, &path).await?;
        let tmp = self.dir.join(format!(".{id}.m3u.cratarr-tmp"));
        tokio::fs::write(&tmp, &new_text).await.with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::set_permissions(&tmp, meta.permissions()).await.ok();
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(anyhow!(e).context(format!("replacing {id}.m3u")).into());
        }
        if let Ok(m) = tokio::fs::metadata(&path).await {
            self.cache
                .write()
                .await
                .insert(id.to_string(), Cached::parse((m.modified().unwrap_or(UNIX_EPOCH), m.len()), &new_text));
        }
        Ok(Toggled { added })
    }

    /// Copy the current file to the backup folder (named `<id>.<millis>.bak`) and keep only the newest few.
    async fn backup(&self, id: &str, path: &Path) -> Result<()> {
        let dir = self.dir.join(BACKUP_DIR);
        tokio::fs::create_dir_all(&dir).await.with_context(|| format!("creating {}", dir.display()))?;
        let mut stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let target = loop {
            let t = dir.join(format!("{id}.{stamp}.bak"));
            if !tokio::fs::try_exists(&t).await.unwrap_or(false) {
                break t;
            }
            stamp += 1;
        };
        tokio::fs::copy(path, &target).await.with_context(|| format!("backing up {id}.m3u"))?;
        let prefix = format!("{id}.");
        let mut mine: Vec<(u128, PathBuf)> = Vec::new();
        let mut rd = tokio::fs::read_dir(&dir).await?;
        while let Some(e) = rd.next_entry().await? {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(mid) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".bak")) else { continue };
            if let Ok(n) = mid.parse::<u128>() {
                mine.push((n, e.path()));
            }
        }
        mine.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
        for (_, old) in mine.into_iter().skip(KEEP_BACKUPS) {
            let _ = tokio::fs::remove_file(old).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("cratarr-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn write(&self, name: &str, content: impl AsRef<[u8]>) {
            std::fs::write(self.0.join(name), content).unwrap();
        }
        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.0.join(name)).unwrap()
        }
        fn store(&self) -> Store {
            Store::new(&self.0)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn view_lists_visible_m3u_files_sorted_with_membership() {
        let d = Dir::new();
        d.write("chill.m3u", "A/1.mp3\nB/2.mp3\n");
        d.write("Analogic.m3u", "B/2.mp3\n");
        d.write("notes.txt", "A/1.mp3\n");
        d.write(".hidden.m3u", "A/1.mp3\n");
        std::fs::create_dir(d.0.join("Some Album (2020)")).unwrap();
        let v = d.store().view(Some("A/1.mp3")).await.unwrap();
        assert_eq!(
            v,
            vec![
                PlaylistView { id: "Analogic".into(), name: "Analogic".into(), member: false, count: 1 },
                PlaylistView { id: "chill".into(), name: "chill".into(), member: true, count: 2 },
            ]
        );
    }

    #[tokio::test]
    async fn no_track_highlights_nothing() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        assert!(d.store().view(None).await.unwrap().iter().all(|p| !p.member));
    }

    #[tokio::test]
    async fn toggle_adds_then_removes_and_the_view_follows() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        let s = d.store();
        assert_eq!(s.toggle("a", "B/2.mp3").await.unwrap(), Toggled { added: true });
        assert_eq!(d.read("a.m3u"), "A/1.mp3\nB/2.mp3\n");
        assert!(s.view(Some("B/2.mp3")).await.unwrap()[0].member);
        assert_eq!(s.toggle("a", "B/2.mp3").await.unwrap(), Toggled { added: false });
        assert_eq!(d.read("a.m3u"), "A/1.mp3\n");
        assert!(!s.view(Some("B/2.mp3")).await.unwrap()[0].member);
    }

    #[tokio::test]
    async fn the_count_follows_edits_and_ignores_comments_and_blanks() {
        let d = Dir::new();
        d.write("a.m3u", "#EXTM3U

A/1.mp3
A/1.mp3
# note
B/2.mp3
");
        let s = d.store();
        assert_eq!(s.view(None).await.unwrap()[0].count, 3, "duplicate lines count, comments and blanks do not");
        s.toggle("a", "C/3.mp3").await.unwrap();
        assert_eq!(s.view(None).await.unwrap()[0].count, 4);
        s.toggle("a", "A/1.mp3").await.unwrap();
        assert_eq!(s.view(None).await.unwrap()[0].count, 2, "removing drops every copy");
    }

    #[tokio::test]
    async fn crlf_files_stay_crlf() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\r\n");
        d.store().toggle("a", "B/2.mp3").await.unwrap();
        assert_eq!(d.read("a.m3u"), "A/1.mp3\r\nB/2.mp3\r\n");
    }

    #[tokio::test]
    async fn every_write_leaves_a_backup_of_the_previous_content() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        d.store().toggle("a", "B/2.mp3").await.unwrap();
        let backups: Vec<_> = std::fs::read_dir(d.0.join(BACKUP_DIR)).unwrap().collect();
        assert_eq!(backups.len(), 1);
        let b = std::fs::read_to_string(backups[0].as_ref().unwrap().path()).unwrap();
        assert_eq!(b, "A/1.mp3\n");
    }

    #[tokio::test]
    async fn old_backups_are_pruned() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        let s = d.store();
        for _ in 0..(KEEP_BACKUPS + 6) {
            s.toggle("a", "B/2.mp3").await.unwrap();
        }
        assert_eq!(std::fs::read_dir(d.0.join(BACKUP_DIR)).unwrap().count(), KEEP_BACKUPS);
    }

    #[tokio::test]
    async fn no_temporary_file_is_left_behind() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        d.store().toggle("a", "B/2.mp3").await.unwrap();
        let names: Vec<String> = std::fs::read_dir(&d.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.contains("cratarr-tmp")), "{names:?}");
    }

    #[tokio::test]
    async fn bad_or_unknown_ids_are_refused() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        let s = d.store();
        for id in ["", "../a", "a/b", ".hidden", "nope", "a\\b"] {
            assert!(matches!(s.toggle(id, "X/1.mp3").await, Err(StoreError::Unknown)), "id {id:?}");
        }
        assert_eq!(d.read("a.m3u"), "A/1.mp3\n");
    }

    #[tokio::test]
    async fn outside_edits_are_noticed() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        let s = d.store();
        assert!(s.view(Some("B/2.mp3")).await.unwrap().iter().all(|p| !p.member));
        d.write("a.m3u", "A/1.mp3\nB/2.mp3\n");
        assert!(s.view(Some("B/2.mp3")).await.unwrap()[0].member);
    }

    #[tokio::test]
    async fn an_edit_always_starts_from_the_files_current_content() {
        let d = Dir::new();
        d.write("a.m3u", "A/1.mp3\n");
        let s = d.store();
        s.view(None).await.unwrap();
        d.write("a.m3u", "A/1.mp3\nC/3.mp3\n"); // another tool edits the file after our view
        s.toggle("a", "B/2.mp3").await.unwrap();
        assert_eq!(d.read("a.m3u"), "A/1.mp3\nC/3.mp3\nB/2.mp3\n");
    }

    #[tokio::test]
    async fn files_that_are_not_utf8_are_never_rewritten() {
        let d = Dir::new();
        d.write("a.m3u", [0x41u8, 0x2f, 0xff, 0xfe, b'\n']);
        let err = d.store().toggle("a", "B/2.mp3").await.unwrap_err();
        assert!(err.to_string().contains("UTF-8"));
        assert_eq!(std::fs::read(d.0.join("a.m3u")).unwrap(), vec![0x41u8, 0x2f, 0xff, 0xfe, b'\n']);
    }

    #[tokio::test]
    async fn a_missing_folder_is_an_error_not_an_empty_list() {
        let s = Store::new("/definitely/not/here");
        assert!(s.view(None).await.is_err());
    }
}
