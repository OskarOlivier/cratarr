# Cratarr

A small, self-hosted web app for keeping your music playlists in order. It works alongside your
player (starting with [Jellyfin](https://jellyfin.org)) instead of replacing it.

**Status: early, working prototype.**

## The idea

Players are good at playing music and weak at managing playlists. Cratarr sits beside them:

- see every playlist at once, with the ones that contain the track you are playing highlighted
- add or remove the playing track with one tap
- later: keep playlists healthy when library files are upgraded, renamed or removed

It is deliberately small.

## How it works

- Playlists are plain `.m3u` files in your music folder (the format Jellyfin imports). Cratarr reads and edits the
  files directly, with a backup before every change.
- Jellyfin tells Cratarr what is playing; Cratarr tells Jellyfin when a playlist file changed.
- Tracks outside your library folder (an incoming or sandbox folder, say) can never be added to a playlist.

## Design goals

- Lean: one container, a single static binary, a small memory budget.
- Loosely coupled: talks to Jellyfin through its public API.
- Dependable after months untouched: few dependencies, pinned builds, tests around the parts that matter.

## Configuration

Environment variables: `JELLYFIN_URL`, `JELLYFIN_API_KEY`, `JELLYFIN_USER_ID`, `CRATARR_PLAYLIST_DIR` (required);
`CRATARR_BIND`, `CRATARR_MUSIC_PREFIXES`, `CRATARR_JELLYFIN_PLAYLIST_DIR`, `CRATARR_PLAYER_CLIENT` (optional,
see `server/src/config.rs`).

## Stack

- Core: Rust ([axum](https://github.com/tokio-rs/axum))
- UI: Svelte 5 + Vite, built to static files and embedded in the binary

See [docs/decisions](docs/decisions) for why, and what is planned next.

## License

MIT
