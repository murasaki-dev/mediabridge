export type Media = {url: string; title?: string; thumbnail_url?: string; duration_seconds?: number; uploader?: string; published?: string; avatar_url?: string};
export function webMediaUrl(value: string) {
  try {
    const url = new URL(value);
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : undefined;
  } catch { return undefined; }
}
export type Status = {
  current?: string; title?: string; playback_state?: string; mirroring?: boolean; position_seconds?: number; duration_seconds?: number;
  is_live?: boolean;
  can_seek?: boolean; previous_available?: boolean; volume?: number; artwork_url?: string; queue_items?: Media[]; history_items?: Media[];
  autoplay_enabled?: boolean; playback_error?: string;
};
export async function api<T>(path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const response = await fetch('/api' + path, body === undefined ? {signal} : {
    method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify(body), signal,
  });
  if (!response.ok) {
    const error = await response.json().catch(() => ({})) as {error?: string};
    throw new Error(error.error || `MediaBridge returned ${response.status}.`);
  }
  return response.json() as Promise<T>;
}
export function artwork(item: Media, size: 'mqdefault' | 'hqdefault' = 'mqdefault') {
  try {
    const url = new URL(item.url);
    const id = url.hostname === 'youtu.be' ? url.pathname.slice(1)
      : ['youtube.com','www.youtube.com','m.youtube.com'].includes(url.hostname) ? url.searchParams.get('v') : null;
    if (id && /^[\w-]{11}$/.test(id)) return `https://i.ytimg.com/vi/${id}/${size}.jpg`;
  } catch {}
  return item.thumbnail_url?.startsWith('https://') ? item.thumbnail_url : undefined;
}
export function preference<T>(key: string, fallback: T): T {
  try { return JSON.parse(localStorage.getItem('mb-svelte-' + key) || 'null') ?? fallback; }
  catch { return fallback; }
}
export function store(key: string, value: unknown) {
  try { localStorage.setItem('mb-svelte-' + key, JSON.stringify(value)); } catch {}
}
