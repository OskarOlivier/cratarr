# 0003: Album covers are read from the album folder

- Status: accepted
- Date: 2026-10-12

## Context

The page should show the cover of the album of the playing track, and nothing more (no playlist covers, no
galleries). Curated album folders contain a cover image, so Jellyfin's image API is not needed.

## Decision

`GET /api/cover/{item_id}` resolves the Jellyfin item (the cached lookup used for the playing track), derives its
library entry as in 0002, takes the entry's directory and serves the first existing file of: `cover.jpg`,
`cover.jpeg`, `cover.png`, `folder.jpg`, `folder.jpeg`, `folder.png`, `front.jpg`, `front.png`, matched
case-insensitively. No resizing; the browser scales. The response has the right `Content-Type`, an `ETag` made from
modification time and size, `Cache-Control: public, max-age=86400`, and 304 for a matching `If-None-Match`.
Everything else is a 404 with a short JSON error: unknown item, track outside the library, no cover, files over
10 MB. The file is read once, never kept in memory.

Security rules:

- The item id from the URL is only a lookup key for Jellyfin (and must look like an id); it is never part of a path.
- The directory comes from the library entry, which must contain no `..`, `.` or empty components, no backslash,
  no NUL, and must not be absolute or sit directly in the library root.
- Only the fixed candidate names are ever opened; nothing from the request chooses a file name.
- The music folder, the album folder and the cover file are canonicalised and must lie inside the music folder, so
  a symlink pointing outside is never followed. The cover must be a regular file.

## Consequences

- A replaced cover shows up after the browser's cache expires (up to a day) or on the next ETag check.
- Large covers are sent as they are; the 10 MB cap bounds memory use.
