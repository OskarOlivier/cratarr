export type Playlist = { id: string; name: string; member: boolean; count: number };

export type NowPlaying = {
  id: string;
  name: string;
  artist: string | null;
  album: string | null;
  /** False when the track is outside the curated library, so it cannot be added to a playlist. */
  addable: boolean;
};

export type State = {
  now_playing: NowPlaying | null;
  playlists: Playlist[];
};

export class ApiError extends Error {}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, init);
  } catch {
    throw new ApiError('Cannot reach the Cratarr server');
  }
  if (!res.ok) {
    let message = `Server returned ${res.status}`;
    try {
      const body = await res.json();
      if (typeof body.error === 'string') message = body.error;
    } catch {
      /* keep the generic message */
    }
    throw new ApiError(message);
  }
  return res.json() as Promise<T>;
}

export const getState = () => request<State>('/api/state');

export const toggle = (playlistId: string) =>
  request<State>(`/api/playlists/${encodeURIComponent(playlistId)}/toggle`, { method: 'POST' });
