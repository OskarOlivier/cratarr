# Cratarr

A small, self-hosted web app for keeping your music playlists in order. It works alongside your
player (starting with [Jellyfin](https://jellyfin.org)) instead of replacing it.

**Status: early. Nothing usable yet.**

## The idea

Players are good at playing music and weak at managing playlists. Cratarr sits beside them:

- see every playlist at once, with the ones that contain the track you are playing highlighted
- add or remove the playing track with one tap
- later: keep playlists healthy when library files are upgraded, renamed or removed

It is deliberately small. Version one does the first two things only.

## Design goals

- Lean: one container, a single static binary, a small memory budget.
- Loosely coupled: talks to Jellyfin through its public API; your playlists stay in Jellyfin.
- Dependable after months untouched: few dependencies, pinned builds, tests around the parts that matter.

## Stack

- Core: Rust ([axum](https://github.com/tokio-rs/axum)), SQLite
- UI: Svelte 5 + Vite, built to static files and embedded in the binary

See [docs/decisions](docs/decisions) for why.

## License

MIT
