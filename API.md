# MediaBridge HTTP API

This guide documents the HTTP interface exposed by MediaBridge for dashboards, remote controls, and future integrations. The examples use POSIX shell syntax and `curl`; replace the host and port with the address of the MediaBridge computer.

```sh
BASE_URL="http://mediabridge-host:8765"
```

The default port is `8765`, but it can be changed in MediaBridge settings. The service must be running and reachable from the client. Use the host's LAN address for another device on the network, for example `http://192.168.1.20:8765`.

## Access and security

The HTTP API currently has no authentication or HTTPS. A client that can reach the listening address can control playback and, for settings routes, change server preferences. Keep the service on a trusted network and do not forward its port to the public internet. For untrusted networks, place it behind a VPN or an authenticated HTTPS reverse proxy.

The browser-origin allowlist is a CORS/browser-origin control; it is not API authentication. `curl` requests without an `Origin` header are not subject to that browser check. Settings endpoints reject cross-origin browser requests, but remain callable by any network client that can reach the server.

POST requests with JSON bodies should send `Content-Type: application/json`. Empty-body actions can be sent as `curl -X POST "$BASE_URL/path"`. Successful requests generally return JSON such as `{"ok":true}`. Errors are JSON with an `error` string; the HTTP status code indicates whether the request was rejected or could not be completed.

## Quick start

Play a URL, inspect playback, then pause it:

```sh
curl -sS -X POST "$BASE_URL/play" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=VIDEO_ID"}'

curl -sS "$BASE_URL/status"
curl -sS -X POST "$BASE_URL/pause"
```

## Playback and queue

### Queue item fields

`/play`, `/queue`, `/queue_next`, and the playlist endpoints accept a JSON queue item. Only `url` is required. All other fields are optional.

| Field | Type | Purpose |
| --- | --- | --- |
| `url` | string | Media URL to play or queue. Normal media URLs are validated. `smb://` URLs are supported only with `local:true`. |
| `local` | boolean | `true` selects local mpv playback (with yt-dlp for supported sites); otherwise playback uses the browser path. Defaults to `false`. |
| `quality` | string | For local mpv playback, use `"best"` or a vertical resolution such as `"720"` or `"1080"`. The allowed numeric range is 144–4320. |
| `screen` | integer | Optional zero-based display index, 0–32. Platform and desktop support determine whether placement is honored. |
| `subtitles` | boolean | Per-item subtitle preference. |
| `browser` | string | Optional browser executable/name for browser playback. The managed LibreWolf browser is used by default on supported builds. |
| `browser_args` | array of strings | Optional extra browser arguments when selecting a browser. |
| `youtube_mode` | string | YouTube browser mode; `"regular"` is the default and `"tv"` selects the TV/Leanback view. |
| `youtube_guest` | boolean | Request the YouTube guest experience in browser playback. |
| `twitch_theater` | boolean | Request Twitch theater mode in browser playback. |
| `title` | string | Optional display title/metadata hint. |
| `uploader` | string | Optional artist/uploader metadata hint. |
| `thumbnail_url` | string | Optional thumbnail/artwork URL. |
| `channel_avatar_url` | string | Optional channel avatar URL. |
| `channel_banner_url` | string | Optional channel banner URL. |
| `autoplay` | boolean | Optional server autoplay switch. When true, MediaBridge expands the current YouTube video's generated Mix in the background after the explicit queue runs out, then appends unseen videos to the queue. |
| `autoplay_minutes` | integer | Optional timeout for `autoplay:true`, from 1 to 1440 minutes. Omit for no timeout. |

`POST /autoplay` also turns the server autoplay switch on or off at any time. Send `{"enabled":true,"duration_minutes":30}` to enable it for 30 minutes, or `{"enabled":false}` to disable it. `GET /status` reports `autoplay_enabled` and `autoplay_until_epoch_ms`. Generated Mix entries appear in `queue_items` with `autoplay_generated:true`; new explicit entries are placed before them.

Applications can inspect the waiting queue through `GET /status`. Use `queue_items` when you need the permanent/autoplay distinction; the simpler `queued` field contains URLs only. In each `queue_items` entry, `autoplay_generated:true` identifies a temporary YouTube Mix item that will be removed if autoplay is turned off. `autoplay_generated:false` identifies a permanent queue item, including Mix entries promoted with `POST /queue/promote`.

### Play or enqueue an item

`POST /play` stops the current item, clears the waiting queue, and starts the submitted item. The old item is retained in playback history for `/previous`.

```sh
curl -sS -X POST "$BASE_URL/play" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=VIDEO_ID","local":true,"quality":"1080"}'
```

`POST /queue` appends an item after the existing queue. If nothing is playing, it starts the submitted item immediately.

```sh
curl -sS -X POST "$BASE_URL/queue" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=NEXT_ID"}'
```

`POST /queue_next` inserts the item at the front of the waiting queue. If nothing is playing, it starts immediately.

```sh
curl -sS -X POST "$BASE_URL/queue_next" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/video.mp4","local":true}'
```

### YouTube playlists

These routes require a YouTube playlist URL with a `list` query parameter. MediaBridge uses yt-dlp to expand the playlist into individual video URLs. The response's `added` count is the number of available entries found.

`POST /play_playlist` replaces current playback and the queue, starts the playlist's first available video, and queues the remaining videos in playlist order.

```sh
curl -sS -X POST "$BASE_URL/play_playlist" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/playlist?list=PLAYLIST_ID"}'
```

`POST /queue_playlist` appends all playlist videos to the waiting queue. If playback is idle, the first available video starts immediately.

```sh
curl -sS -X POST "$BASE_URL/queue_playlist" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/playlist?list=PLAYLIST_ID","local":true}'
```

`POST /queue_playlist_next` inserts all playlist videos at the front of the queue, preserving their playlist order. If playback is idle, the first video starts immediately.

```sh
curl -sS -X POST "$BASE_URL/queue_playlist_next" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/playlist?list=PLAYLIST_ID"}'
```

### Inspect and edit the queue

`GET /status` returns playback state and the current queue. The `queued` array contains queued URLs; `queue_items` adds metadata and playback options for each waiting entry. See [Status response](#status-response).

`POST /queue/remove` removes a waiting entry by its zero-based `index` in the queue. The currently playing item is not part of this index space.

```sh
curl -sS -X POST "$BASE_URL/queue/remove" \
  -H 'Content-Type: application/json' -d '{"index":0}'
```

`POST /queue/move` reorders a waiting entry. Both `index` and `to` are zero-based positions in the queue.

```sh
curl -sS -X POST "$BASE_URL/queue/move" \
  -H 'Content-Type: application/json' -d '{"index":2,"to":0}'
```

`POST /queue/promote` turns a generated YouTube Mix entry into a permanent queue entry. Promoted entries stay queued when autoplay is disabled.

```sh
curl -sS -X POST "$BASE_URL/queue/promote" \
  -H 'Content-Type: application/json' -d '{"index":0}'
```

### Playback controls

All controls below use `POST`. They control the active mpv item or managed browser playback where supported. If no controllable player is active, the server returns a conflict error.

| Route | Description |
| --- | --- |
| `/pause` | Pause the current media. |
| `/resume` | Resume the current media. |
| `/play_pause` | Toggle between paused and playing. |
| `/rewind` | Seek backward by the player's default step. |
| `/forward` | Seek forward by the player's default step. |
| `/volume_up` | Increase volume by 0.05. |
| `/volume_down` | Decrease volume by 0.05. |

Example:

```sh
curl -sS -X POST "$BASE_URL/pause"
curl -sS -X POST "$BASE_URL/resume"
curl -sS -X POST "$BASE_URL/play_pause"
curl -sS -X POST "$BASE_URL/rewind"
curl -sS -X POST "$BASE_URL/forward"
curl -sS -X POST "$BASE_URL/volume_up"
curl -sS -X POST "$BASE_URL/volume_down"
```

`POST /seek` moves to an absolute position in seconds. `position` must be a finite, non-negative number.

```sh
curl -sS -X POST "$BASE_URL/seek" \
  -H 'Content-Type: application/json' -d '{"position":125.5}'
```

`POST /volume` sets volume from `0.0` (mute) through `1.0` (maximum).

```sh
curl -sS -X POST "$BASE_URL/volume" \
  -H 'Content-Type: application/json' -d '{"volume":0.5}'
```

`POST /previous` starts the previous item and places the interrupted current item at the front of the queue. Returns `409` if there is no history.

`POST /next` starts the first queued item and adds the current item to history. Returns `409` if the queue is empty.

`POST /close` is an abort action: it stops playback and mirroring, disables autoplay and its timeout, clears the queue and history, and cancels active background yt-dlp lookups. It returns `409` if playback state was cleared but the managed LibreWolf window could not be confirmed closed.

```sh
curl -sS -X POST "$BASE_URL/previous"
curl -sS -X POST "$BASE_URL/next"
curl -sS -X POST "$BASE_URL/close"
```

### Host mouse control

`POST /mouse` sends pointer input to the MediaBridge computer. This is useful for integrations that provide a remote touchpad. Absolute `x`/`y` coordinates are in the range 0–32767. Relative `dx`/`dy` values may be signed and range from -32767 to 32767. `button` is `left`, `right`, or `middle` and defaults to `left`.

Supported actions are `move`, `click`, `move_relative`, and `click_current`.

```sh
curl -sS -X POST "$BASE_URL/mouse" \
  -H 'Content-Type: application/json' \
  -d '{"x":16000,"y":12000,"action":"click","button":"left"}'

curl -sS -X POST "$BASE_URL/mouse" \
  -H 'Content-Type: application/json' \
  -d '{"dx":-40,"dy":15,"action":"move_relative"}'
```

On Linux, pointer control depends on the active desktop session and its `xdotool` (X11) or `ydotool` (Wayland) setup.

## Playback status response

`GET /status` returns a JSON object with these fields (values can be `null` when the player does not provide them):

The response exposes the current item separately from the waiting queue. `current` is only a URL and does not include whether that playing item was generated by autoplay. `queue_items` covers waiting items only, so applications can identify temporary and permanent entries there, but the API does not currently return a single ordered playlist containing the current item and its origin flag.

| Field | Meaning |
| --- | --- |
| `current` | Current item's URL, if any. |
| `queued` | Array of waiting item URLs. |
| `queue_items` | Array of waiting item objects with URL, metadata, playback options, and `autoplay_generated`. `true` means temporary Mix-generated; `false` means permanent. |
| `playing` | `true` when the state is reported as playing. |
| `mirroring` | Whether a Moonlight process is active. |
| `playback_state` | Player state such as `playing`, `paused`, `idle`, `mirroring`, or `unknown`. |
| `playback_error` | Current playback error, if reported. |
| `title`, `artist`, `album` | Current media metadata. |
| `artwork_url` | Current artwork/thumbnail URL. |
| `channel_avatar_url`, `channel_banner_url` | Channel artwork URLs when available. |
| `media_url` | Active media URL reported by the player, if available. |
| `position_seconds`, `duration_seconds` | Current position and total duration in seconds. |
| `volume` | Volume from 0.0 to 1.0 when available. |
| `can_seek` | Whether a usable duration is available for seeking. |
| `previous_available` | Whether a previous item exists in playback history. |
| `autoplay_enabled` | Whether post-queue autoplay is enabled. |
| `autoplay_until_epoch_ms` | Optional expiration time as Unix epoch milliseconds. `null` means no timeout. |

Example:

```sh
curl -sS "$BASE_URL/status"
```

## Link metadata resolution

`POST /resolve` asks yt-dlp to inspect a media URL and returns supported metadata such as a canonical URL, title, uploader, thumbnail, channel artwork, duration, and live status. It does not start playback. This can take time for some sites.

```sh
curl -sS -X POST "$BASE_URL/resolve" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=VIDEO_ID"}'
```

`POST /resolve_async` queues a metadata lookup for the server's background resolver and returns promptly with `202 Accepted` and `{"ok":true}`. Duplicate pending URLs are not queued twice. The current API does not expose a separate operation-id or result-polling route for this request; use `/resolve` when the caller needs the metadata response.

```sh
curl -sS -X POST "$BASE_URL/resolve_async" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=VIDEO_ID"}'
```

## Server and playback settings

These routes are intended for local dashboard/configuration use. They accept `Origin`-less HTTP clients, but browsers cannot use them cross-origin.

`GET /preferences` returns saved playback defaults: `default_screen`, `default_quality`, `subtitles`, `preferred_language`, `cookies_browser`, and `cookies_profile`.

`POST /settings/preferences` saves those defaults. The request accepts the same fields; omitted fields take their defaults (`null`, `false`, `"auto"`, or `null`, depending on the field), so send the full desired settings object when updating preferences.

```sh
curl -sS "$BASE_URL/preferences"
curl -sS -X POST "$BASE_URL/settings/preferences" \
  -H 'Content-Type: application/json' \
  -d '{"default_screen":1,"default_quality":"1080","subtitles":true,"preferred_language":"en","cookies_browser":"librewolf","cookies_profile":null}'
```

`default_screen` is `null` or 0–32. `default_quality` is `null`, `"best"`, or a numeric resolution from 144–4320. `cookies_browser` can be `auto`, `off`, `librewolf`, `chrome`, `chromium`, `edge`, `firefox`, `brave`, `opera`, `vivaldi`, or `whale`. A cookie profile is optional and must not contain commas or line breaks.

`GET /platform` returns the server operating system, for example `{"os":"windows"}` or `{"os":"linux"}`.

`GET /settings/port` returns the listening port. `POST /settings/port` saves a new port and asks the server to restart its listener. The response is `202` with `{"port":PORT,"restart":true}`; the active connection may be interrupted as the server restarts. Port must be from 1–65535 and available.

```sh
curl -sS "$BASE_URL/settings/port"
curl -sS -X POST "$BASE_URL/settings/port" \
  -H 'Content-Type: application/json' -d '{"port":9000}'
```

`GET /settings/remote-origins` reads the browser CORS allowlist. `POST /settings/remote-origins` replaces the list. Each entry must be a web origin such as `https://party.example`, without a path. An empty array clears it.

```sh
curl -sS "$BASE_URL/settings/remote-origins"
curl -sS -X POST "$BASE_URL/settings/remote-origins" \
  -H 'Content-Type: application/json' \
  -d '{"remote_origin_servers":["https://party.example"]}'
```

### Check local browser-cookie access

`POST /settings/cookies/check` checks whether the MediaBridge computer can use a browser's local cookies for YouTube or Twitch. It does not accept cookie contents from the remote client. `cookies_browser` is required in practice and may be `auto`, `librewolf`, `chrome`, `chromium`, `edge`, `firefox`, `brave`, `opera`, `vivaldi`, or `whale`; `cookies_profile` is optional; `site` is `youtube` or `twitch` and defaults to `youtube`. Success returns the check result as JSON. Failure to access/decrypt cookies or confirm the account returns `422` with an `error` message.

```sh
curl -sS -X POST "$BASE_URL/settings/cookies/check" \
  -H 'Content-Type: application/json' \
  -d '{"cookies_browser":"librewolf","cookies_profile":null,"site":"youtube"}'
```

Cookie access is performed on the server host, using the account and browser profile available to the MediaBridge process.

## Sunshine / Moonlight

`POST /sunshine/pair` starts Moonlight's pairing flow for a Sunshine host name or IP. Moonlight displays its pairing PIN. Enter that PIN into Sunshine's pairing prompt; MediaBridge verifies the pairing afterward.

```sh
curl -sS -X POST "$BASE_URL/sunshine/pair" \
  -H 'Content-Type: application/json' -d '{"host":"192.168.1.50"}'
```

`GET /sunshine/pair/status` reports `idle`, `waiting`, `paired`, or `failed`, and may include `host`, `message`, or `error`. Poll it while the user completes PIN confirmation.

```sh
curl -sS "$BASE_URL/sunshine/pair/status"
```

`POST /sunshine` stops current media, clears the queue, and launches Moonlight streaming to the requested host and app. The app defaults to `Desktop`.

```sh
curl -sS -X POST "$BASE_URL/sunshine" \
  -H 'Content-Type: application/json' -d '{"host":"192.168.1.50","app":"Desktop"}'
```

## Dashboard routes

`GET /` and `GET /index.html` serve the built-in MediaBridge dashboard HTML page. Integrations usually use the JSON API routes above instead.

## Errors and status codes

Common responses include:

| Status | Meaning |
| --- | --- |
| `200` | Request completed. |
| `202` | Playback, playlist expansion, pairing, or restart was accepted/started. |
| `204` | CORS preflight response. |
| `400` | Invalid JSON, missing/invalid fields, or invalid URL/action. |
| `403` | Browser `Origin` was denied by the origin rules. |
| `404` | Unknown route or a queue index was not found. |
| `409` | Requested operation is unavailable in the current state, such as empty queue, no previous item, active restart, or an unavailable port. |
| `422` | A media URL could not be resolved or cookie access could not be confirmed. |
| `500` | A player/process/configuration operation failed. |
| `503` | The asynchronous metadata resolver is unavailable. |

Error bodies generally use this shape:

```json
{"error":"Human-readable explanation"}
```
