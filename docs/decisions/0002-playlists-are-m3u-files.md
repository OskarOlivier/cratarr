# 0002: Playlists are `.m3u` files; Jellyfin supplies what is playing

- Status: accepted (supersedes the playlist handling in 0001)
- Date: 2026-10-12

## Context

0001 assumed playlists lived inside Jellyfin and could be edited through its API. Measuring a real library showed
otherwise:

- The playlists are plain `.m3u` files in the root of the music library. Jellyfin only imports them: each one is
  a database item pointing at the file, and the entries are read from the file at scan time.
- Jellyfin cannot edit them. Imported playlists have no owner, so adding an entry returns 403 even for an
  administrator, and its view of the music folder may be read-only.
- Reading a playlist through Jellyfin is slow (about 1 ms per entry just to resolve the playlist, plus about
  8 ms per entry returned; roughly two minutes for 13,000 entries), which forced a background index and a cache.
- Jellyfin's session record can be stale with some clients (an old track kept as "paused"), while its activity
  log correctly records the newest audio playback.

## Decision

- The `.m3u` files are the source of truth. Cratarr reads them directly (milliseconds) and edits them directly.
- An edit re-reads the file, backs it up (`.cratarr-backups/`, newest 20 kept), writes a temporary file next to
  it and renames it into place. Only the one line is added or removed; comments, line endings and odd entries
  are preserved. Files that are not valid UTF-8 are never rewritten.
- Matching is by library-relative path (`Folder/File.mp3`). A track outside the configured library prefixes
  (for example an incoming folder) has no such path and can never be added.
- "What is playing" comes from Jellyfin's activity log (the newest audio start without a later stop), with the
  session record as a fallback.
- After a write, Cratarr asks Jellyfin to refresh that one playlist item (best effort) so players see the change.
  Measured on a real server: a "file changed" notice does nothing; the `ValidationOnly` and `Default` refresh
  modes do nothing; plain `FullRefresh` picks up added entries but keeps removed ones; `FullRefresh` with
  `ReplaceAllMetadata=true` handles both and the entry count followed the file within seconds.
- The container needs read-write access to the playlist folder, and only `*.m3u` files directly in it are touched.

## Consequences

- No index, cache or background sync; the first view of 67 playlists takes well under a second.
- Entries are matched by path, so replacing a file (for example a FLAC upgrade of an MP3) leaves the playlist
  entry pointing at the old name. Healing entries is a later feature.
- Backups of the `.m3u` files are not covered by backing up Jellyfin's database; they need their own backup.

## Roadmap

Two-way sync with Jellyfin's native playlists, so playlists created in Jellyfin are not bypassed, comes after the
other planned features.
