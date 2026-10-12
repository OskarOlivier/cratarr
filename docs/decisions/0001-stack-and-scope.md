# 0001: Scope and stack for version one

- Status: accepted
- Date: 2026-10-12

## Context

Jellyfin and most players manage playlists poorly. There is no fast way to see which playlists contain the
track that is playing, and no way to keep playlist entries valid when library files are replaced.

## Decision

- **Scope of v1:** list every playlist with the playing track's memberships highlighted, and add or remove the
  playing track. Tracks from an optional "incoming" library path are excluded. Playlist repair, health reports,
  voice control and audio analysis come later.
- **Playlists stay in Jellyfin.** Cratarr reads and writes them through the Jellyfin API and keeps its own index
  only for speed.
- **Core in Rust** (axum, reqwest, serde, SQLite): small memory footprint, a single static binary, and a strict
  compiler that acts as a reviewer for generated or rarely touched code.
- **UI in Svelte 5 with Vite**, built to static files and embedded in the binary: fine-grained reactivity suits
  a page of many chips that flip as the track changes, and there is no UI runtime to maintain.
- **Tests from the start:** matching rules, the incoming-path exclusion, the Jellyfin client against saved real
  responses, and a startup self-check that fails loudly when Jellyfin's API shape changes.

## Consequences

- One small container; a Jellyfin API key is the only credential it needs.
- Slower iteration than a scripting language; accepted for the type safety and footprint.
- Jellyfin API changes are the main long-term risk; the saved-response tests and startup check target it.
