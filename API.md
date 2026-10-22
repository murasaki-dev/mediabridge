# MediaBridge HTTP API

This guide documents the HTTP interface exposed by MediaBridge for dashboards, remote controls, and future integrations. The examples use POSIX shell syntax and `curl`; replace the host and port with the address of the MediaBridge computer.

```sh
BASE_URL="http://mediabridge-host:8765"
```

The default port is `8765`, but it can be changed in MediaBridge settings. The service must be running and reachable from the client. Use the host's LAN address for another device on the network, for example `http://192.168.1.20:8765`.

## SMB credential providers

### What a provider is, and what is included

A **provider** is a separate trusted component that receives a share reference and file URL, checks access, and returns either login credentials or a private streaming URL. A **relay** reads the SMB file and serves it to mpv over local HTTP. A provider can use a relay, but returning credentials alone does not provide a relay.

Standalone MediaBridge includes the provider connection code and private mpv launch mechanism. **It does not include a native saved-login system or a ready-to-run native credential provider.** Setting an environment variable or inventing a share ID does not create credentials. An operator must supply a provider implementation, or run the compatible Baka-Inu service, before authenticated playback can work.

| Playback arrangement | What must be supplied |
| --- | --- |
| Guest SMB, no `smb_share_id` | A guest-readable share and an mpv build with SMB protocol support. No credential service is called. |
| Provider returns username/password | A trusted native provider or Baka-Inu credential service, authorized share mapping, saved/session credentials, and an SMB-enabled mpv build. |
| Provider returns `playback_url` | A compatible provider and seekable loopback relay. The relay reads SMB; mpv plays HTTP and does not need native SMB protocol support. This also supports guest shares through a provider reference. |

Baka-Inu supplies its own share bookmarks, login dialogs, credential storage, access checks, and SMB relay. Those are separate from this Rust API. Standalone MediaBridge currently has no HTTP endpoints for listing shares, browsing directories, creating bookmarks, saving/deleting SMB passwords, or issuing share IDs. Its native connection point allows another component to supply those capabilities; it does not implement them automatically.

### Client playback requests

SMB URLs stay credential-free in requests, queue state, history, and public status. Set `local:true` and an opaque 24-hex `smb_share_id` to request provider-backed playback. Without a reference, MediaBridge uses direct guest SMB. SMB metadata uses the filename. Playback disables mpv watch-later positions and starts at zero unless `start_position_seconds` specifies a saved position, including one captured by `/play_interrupt`.

```sh
# Direct guest share, using an SMB-enabled mpv build:
curl -sS -X POST "$BASE_URL/queue" \
  -H 'Content-Type: application/json' \
  -d '{"url":"smb://fileserver/media/movie.mp4","local":true,"title":"movie.mp4"}'

# Provider-backed playback; this ID must already be recognized by the provider:
curl -sS -X POST "$BASE_URL/play_preserve_queue" \
  -H 'Content-Type: application/json' \
  -d '{"url":"smb://fileserver/media/movie.mp4","local":true,"title":"movie.mp4","smb_share_id":"0123456789abcdef01234567"}'
```

Use `smb://server/share/path/to/file` with URL-encoded spaces. Submitted SMB URLs must have a server, share, and file path, and must not contain embedded usernames/passwords, query strings, fragments, or control characters. SMB is accepted only for local mpv playback; the public `/resolve` and `/resolve_async` routes accept HTTP(S) links. Internal SMB metadata lookup uses the filename and does not launch yt-dlp.

The share ID is a provider lookup key, not an authentication token. MediaBridge checks that it has 24 hexadecimal characters, but the provider must decide which URLs it authorizes. A client-facing integration should select/overwrite this field from its own authorized share mapping. Public queue/history item responses omit the reference; clients needing authenticated history replay must obtain the appropriate reference again from their integration.

### Native executable provider contract

For native deployments, set `MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER` to a trusted executable path (no shell command or arguments). MediaBridge sends one JSON object to stdin, closes it, and expects JSON on stdout followed by a successful exit:

```json
{"share_id":"0123456789abcdef01234567","url":"smb://server/share/folder/movie.mp4"}
```

The provider must validate that the requested file is inside the authorized share root before retrieving secrets from its OS credential store or other protected storage. It returns `{"username":"user","password":"...","domain":"PC"}` (domain optional), or `{"playback_url":"http://127.0.0.1:8081/api/internal/smb-media/<64-hex-capability>"}`. A relay must implement file serving and seeking; MediaBridge does not host that service. The relay URL must match the configured room port, exact path prefix, and 64 hexadecimal characters without a query or fragment. Provider requests are capped at 16 KiB, responses at 32 KiB, with a five-second execution deadline and process cleanup. Provider stderr and raw failures are suppressed to protect secrets. Configure an executable that exits promptly and does not leave detached descendants.

Example host configuration, after installing your own provider executable:

```powershell
# Windows: set these in the process that starts MediaBridge.
$env:MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER = 'C:\MediaBridge\smb-provider.exe'
$env:MEDIABRIDGE_SMB_ROOM_PORT = '8081' # also constrains relay URLs
& 'C:\MediaBridge\mediabridge.exe'
```

```sh
# Linux: these paths are examples, not bundled files.
export MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER=/opt/mediabridge/smb-provider
export MEDIABRIDGE_SMB_ROOM_PORT=8081
mediabridge
```

These variables belong to the MediaBridge host process, not a remote API client, and are read when playback starts. A queued item may be accepted before its provider is contacted. Do not put a password in the variable, executable arguments, or public queue JSON.

Provider implementation sequence:

1. Read the bounded JSON request from stdin until EOF and look up `share_id` in a server-controlled mapping.
2. Validate/canonicalize the requested SMB location against that share's allowed root, including traversal and encoded-path handling. Reject unknown IDs and URLs outside the root before releasing credentials or creating a relay capability.
3. Retrieve the authorized login from protected storage, or create a private relay capability for that exact file. The provider owns storage encryption/key handling, OS-store access, and credential lifetime; MediaBridge supplies none of these.
4. Write only the JSON response to stdout and exit with status zero. Reject requests with a nonzero exit. Raw provider stderr and nonzero-exit text are not forwarded to clients; callers receive a generic provider failure.

Username/password response example (illustrative values only):

```json
{"username":"media-user","password":"example-only","domain":"FILESERVER"}
```

`username` and `password` must be strings; `domain` is optional. MediaBridge percent-encodes these fields when constructing the in-memory SMB URL. If a string `playback_url` is present, it takes precedence over credential fields and must pass relay validation. `localhost`, a different port, another path, or an additional query/fragment is rejected; the accepted host is exactly `127.0.0.1`. The relay must already be running and should support GET/HEAD and byte-range requests for seeking and MP4 metadata reads.

### Baka-Inu loopback adapter

Alternatively, Baka-Inu's adapter reads `MEDIABRIDGE_SMB_CONFIG_DIR/smb-playback.token` (64 hex characters), then calls the protected loopback credential endpoint on `MEDIABRIDGE_SMB_ROOM_PORT` (default 8081), with a Bearer token, share ID, and encoded original URL. Connect/write timeouts are three seconds, the socket read timeout is five seconds, and the complete response is capped at 32 KiB. The room service owns token verification, share-boundary authorization, storage, and relay capabilities. The native executable takes precedence when configured.

Configure `MEDIABRIDGE_SMB_CONFIG_DIR` to the directory containing the service's token, and run the service in the same loopback network namespace as MediaBridge. The adapter sends:

```http
GET /api/internal/smb-credentials?share=0123456789abcdef01234567&url=smb%3A%2F%2Ffileserver%2Fmedia%2Fmovie.mp4 HTTP/1.1
Host: 127.0.0.1:8081
Authorization: Bearer <private-64-hex-token>
Connection: close
```

This endpoint belongs to the companion service; it is not a MediaBridge route. The service must verify the loopback peer, token, share authorization, and file boundary. The current adapter expects an `HTTP/1.1 200` response with a directly readable JSON body; it does not decode chunked transfer encoding. The 32 KiB bound includes HTTP headers. Socket timeouts are not a single end-to-end request deadline. If the native executable is configured and fails, MediaBridge reports that failure rather than falling back to the loopback service.

### Privacy, diagnostics, and remaining setup

Private authenticated or relay URLs go only to mpv's stdin playlist, with a forced safe title. They are excluded from argv and raw player logs; only static error categories are reported. Direct SMB needs a compatible mpv build; installing an extra FFmpeg package alone does not guarantee protocol support. Standalone MediaBridge does not include Baka-Inu's SMB browser, bookmark store, credential UI, or relay server.

The stdin input is a one-item `#EXTM3U` playlist; terminal input is disabled. Both private player output streams are drained without printing raw lines. Static diagnostic categories cover unsupported protocols, rejected login, denied access, missing files, rejected mpv options, unrecognized media/playlist input, and connection failures. Renderer/audio fallback messages are excluded from SMB file-error classification. These messages can appear in `playback_error`; a later unsuccessful process exit preserves an existing detailed error. Safe launch logs retain the original SMB URL, provider-reference presence, playback epoch, and requested start position; EOF logs omit the private URL.

| Failure | What to check |
| --- | --- |
| Credential service is not configured | Supply a native provider executable or configure the loopback service directory. A share ID alone cannot supply credentials. |
| Invalid credential reference | Use a provider-recognized 24-hex ID on an SMB item with `local:true`. |
| Provider timeout/nonzero exit/invalid response | Check the provider's own protected diagnostics, URL authorization, exit behavior, response schema, and size. |
| Unreachable loopback service/token failure | Check the service process, port, shared network namespace, token file, and service authorization. |
| Invalid relay URL/capability | Match the exact configured port/path and 64-hex capability format. |
| Unsupported SMB protocol | Use a compatible relay or an SMB-enabled mpv build. Returning credentials does not add SMB support to mpv. |
| Login/access/file failure | Check credentials, share-root authorization, file permissions, and whether the file still exists. |

Authenticated SMB playback in this standalone checkout has not been exercised end to end against a live share. Baka-Inu's separate relay verification does not establish that a new native provider or this host's mpv configuration works. Provider setup and live playback verification remain deployment work.

## Changes for existing integrations

| Area | Current behavior |
| --- | --- |
| Playback defaults | 720p/H.264 initially. Loading configuration changes missing/null or saved `best` default quality to 720p. Per-item `quality:"best"` remains uncapped. |
| Manual queue additions | Preserve generated suggestions. Ordinary additions precede the first suggestion; `/queue` with `at_end:true` appends literally last. Next-item/playlist operations preserve the rest of the queue. |
| New playback routes | `/play_preserve_queue` replaces only current playback; `/queue_batch` adds an ordered selection. |
| Queue/history data | Duration, metadata failure, and caller attribution are exposed; `history_items` is newest first. `/close` preserves in-memory session history. |
| Autoplay | Immediate live candidate scan, retry cooldown, cancellation generation checks, progress/errors, and `/autoplay/error/clear`. |
| Cookie usage | Configured browser cookies now also apply to background YouTube metadata, playlist, and Mix lookups. |
| Decode/logging | Hardware decode selection/reporting, Linux stderr log mirroring, and Linux LibreWolf sign-in launcher/PID/stderr diagnostics. |
| SMB | Provider-backed private playback, optional HTTP relay, filename metadata, explicit start/resume positions, and safe error categories. The native login store and relay remain external. |

No Rust dependencies were added or upgraded for these changes. Submitter IDs remain caller-supplied metadata; room authentication and roles belong to the integrating application. In particular, MediaBridge does not implement Baka-Inu's admin-only error dismissal or SMB browsing permissions.

### Runtime environment controls

| Variable | Purpose |
| --- | --- |
| `MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER` | Trusted native provider executable path. Takes precedence over the loopback adapter. |
| `MEDIABRIDGE_SMB_CONFIG_DIR` | Loopback adapter token directory. Used when no native executable is configured. |
| `MEDIABRIDGE_SMB_ROOM_PORT` | Loopback credential service port and accepted relay URL port; defaults to 8081 if absent or not parseable as `u16`. |
| `MEDIABRIDGE_MPV_HWDEC` | mpv hardware decoding option; defaults to `auto`. This controls decoding, not desktop-stream encoding. |
| `MEDIABRIDGE_LOG_STDERR` | On Linux, `1` mirrors file diagnostics to stderr, useful for container logs. |
| `XDG_STATE_HOME` | On Linux, selects the state root used by MediaBridge's log path. |

The Linux installer creates a private, preserved `~/.config/mediabridge/runtime.env` (or the corresponding `XDG_CONFIG_HOME` location). Its generated launcher loads this trusted shell configuration for desktop/service launches and forwards supported variables into Distrobox. This supplies a persistent place to configure a provider; it does not install one. See [Linux runtime settings](docs/INSTALLATION.md#linux-paths-and-runtime-settings) for setup and restart commands.

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

`/play`, `/play_preserve_queue`, `/play_interrupt`, `/queue`, `/queue_next`, and the playlist endpoints accept a JSON queue item. Only `url` is required. All other fields are optional.

| Field | Type | Purpose |
| --- | --- | --- |
| `url` | string | HTTP(S) URL to open or play. Browser playback can open any website; `smb://` URLs are supported only with `local:true`. |
| `local` | boolean | `true` selects local mpv playback (with yt-dlp for supported sites); otherwise playback uses the browser path. Defaults to `false`. |
| `quality` | string | For local mpv playback, use `"best"` or a vertical resolution such as `"720"` or `"1080"`. The allowed numeric range is 144–4320. |
| `start_position_seconds` | number or null | Optional finite, non-negative start position in seconds. Used to restore interrupted videos. Exposed on queued/history items until consumed at launch. Browser restoration requires a controllable, seekable video. |
| `screen` | integer | Optional zero-based display index, 0–32. Platform and desktop support determine whether placement is honored. |
| `subtitles` | boolean | Per-item subtitle preference. |
| `load_multiple_audio_tracks` | boolean | Optional per-video override of `track_preferences.load_multiple_audio_tracks`. False requests only extracted original/default audio; true exposes alternate extracted formats for track selection. Applies to mpv and takes effect at launch. Embedded tracks in one file remain visible either way. |
| `browser` | string | Optional browser executable/name for browser playback, such as `"librewolf"`. Set it to `"librewolf"` to use the managed profile, which can open any HTTP(S) website on supported builds. |
| `browser_args` | array of strings | Optional extra browser arguments when selecting a browser. |
| `youtube_mode` | string | YouTube browser mode; `"regular"` is the default and `"tv"` selects the TV/Leanback view. |
| `youtube_guest` | boolean | Request the YouTube guest experience in browser playback. |
| `twitch_theater` | boolean | Request Twitch theater mode in browser playback. |
| `title` | string | Optional display title/metadata hint. |
| `queued_by_user_id` | string | Optional opaque caller-supplied attribution; MediaBridge does not authenticate this identity. |
| `duration_seconds` | number | Optional duration hint; successful metadata lookup retains positive finite durations. |
| `metadata_error` | string | Last metadata lookup failure; cleared after a successful lookup. |
| `smb_share_id` | string | Optional 24-hex credential-provider reference for local SMB playback; never a password. See [SMB credential providers](#smb-credential-providers). |
| `uploader` | string | Optional artist/uploader metadata hint. |
| `thumbnail_url` | string | Optional thumbnail/artwork URL. |
| `channel_avatar_url` | string | Optional channel avatar URL. |
| `channel_banner_url` | string | Optional channel banner URL. |
| `autoplay` | boolean | Optional server autoplay switch. Enabling starts a background Mix search immediately; suggestions are appended after explicit queue entries. |
| `autoplay_minutes` | number | Legacy alias for setting the global close countdown in positive minutes, independent of `autoplay`. Decimals are accepted. Prefer `/timer` for new clients. Omission leaves an existing countdown unchanged. |

`POST /autoplay` turns the server autoplay switch on or off. Send `{"enabled":true}` or `{"enabled":false}`. Autoplay has no separate expiry policy. The optional legacy `duration_minutes` field now sets the global close countdown; it does not merely stop Mix generation. Omitted/null `duration_minutes` leaves the countdown unchanged, and disabling autoplay does not cancel it. Use `/timer` to set, replace, or cancel the countdown. `GET /status` reports `autoplay_enabled`. Generated Mix entries appear in `queue_items` with `autoplay_generated:true`; new explicit entries are placed before them.

Enabling immediately tries the current YouTube video and then up to 20 eligible non-generated YouTube queue entries in live order. Non-YouTube entries do not consume those slots. One worker reads up to 40 Mix candidates and appends up to 20 unseen suggestions. Existing suggestions stop further expansion. Unavailable/duplicate-only seeds advance the scan; operational failures retain a diagnostic and retry after a 60-second cooldown. Cancelling invalidates worker generations. Background cancellable commands have a 90-second per-command deadline; ordinary playlist expansion and uncancelled resolver calls retain their existing execution paths. Mix availability is not guaranteed.

`POST /autoplay/error/clear` returns `{"ok":true}` and dismisses only the diagnostic. It leaves the worker, retry cooldown, queue, and autoplay setting intact. MediaBridge has no role checks; an integration must enforce its own permissions.

Applications can inspect the waiting queue through `GET /status`. Use `queue_items` when you need the permanent/autoplay distinction; the simpler `queued` field contains URLs only. In each `queue_items` entry, `autoplay_generated:true` identifies a temporary YouTube Mix item that will be removed if autoplay is turned off. `autoplay_generated:false` identifies a permanent queue item, including Mix entries promoted with `POST /queue/promote`.

`autoplay_generated` is assigned internally and is ignored in submitted item JSON. New suggestions clone the seed's playback settings, but clear submitter identity, duration, metadata/error/artwork, and SMB provider reference. The API does not assign display names to submitter IDs.

### Autoplay scan, progress, and dismissal

```sh
curl -sS -X POST "$BASE_URL/autoplay" \
  -H 'Content-Type: application/json' \
  -d '{"enabled":true}'

curl -sS "$BASE_URL/status"
curl -sS -X POST "$BASE_URL/autoplay/error/clear"

curl -sS -X POST "$BASE_URL/autoplay" \
  -H 'Content-Type: application/json' -d '{"enabled":false}'
```

The scan tracks checked video IDs for the current playback root. Candidate selection reads the live queue on each dispatch, so newly added eligible entries can participate in the first 20 eligible slots. Changing the root clears the scan cache. A result can be used while its seed remains current or queued. Suggestions are filtered against current playback, queue, session history, and seen IDs. Turning autoplay off cancels the worker, invalidates older results, removes unplayed generated suggestions, and releases their seen markers so a later retry can suggest them again. Toggling resets the retry cooldown and checked-seed scan state.

`autoplay_status` is display text, not a stable enum or structured progress object. Examples include `Checking current video · 2 queued candidates (max 20)`, `Checking queue item 23 · 2 queued candidates (max 20)`, and `No Mix available · waiting for new videos`. Queue positions are one-based positions in the full live queue; they can exceed 20 when other entries precede the eligible seeds. Clients should use `autoplay_loading` and `autoplay_error` for behavior rather than parse the display text. Clearing an error does not force a retry; the existing cooldown remains.

Mix extraction first tries the seeded watch URL, then the direct Mix playlist URL. The latter can itself be unavailable. Recognized unavailable-playlist outcomes advance to another seed; other nonzero extractor exits are operational failures and trigger the cooldown. The current implementation also classifies exhausted parse/invalid-JSON attempts as `NO_MIX:` internally. That prefix is an internal classification, not a typed public error contract. These are generated YouTube Mix lookups, not general “Up next” recommendations.

The 90-second deadline is per cancellable subprocess invocation. A two-attempt extraction and a multi-seed scan can take longer overall. There is no total scan deadline or background-output size cap in this implementation.

### Play or enqueue an item

`POST /play` stops the current item, clears the waiting queue, and opens or plays the submitted URL. Browser playback can open regular websites as well as media links; MediaBridge reports the page title when it can inspect the active managed LibreWolf tab. Video controls are available when the page exposes a controllable HTML video. The old item is retained in playback history for `/previous`.

```sh
curl -sS -X POST "$BASE_URL/play" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=VIDEO_ID","local":true,"quality":"1080"}'
```

To open a regular website in the managed LibreWolf browser, set `browser` explicitly:

```sh
curl -sS -X POST "$BASE_URL/play" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/","browser":"librewolf"}'
```

`POST /queue` inserts before the first generated suggestion, preserving the existing queue. Send `at_end:true` to append after the entire queue. If nothing is playing, it starts the submitted item immediately.

`POST /play_preserve_queue` replaces current playback and archives the former item, while preserving all waiting entries. It accepts the same body as `/play`.

### Interrupt playback and resume it next

`POST /play_interrupt` accepts the same queue-item body as `/play`. With a seekable current video, it captures the reported playback position, pauses that video, and inserts a copy at the very front of the waiting queue with `start_position_seconds` set to the captured value. It then replaces current playback with the submitted video. All other queue entries retain their order. When the new video ends, or `/next` is requested, the interrupted video launches from the saved position.

```sh
curl -sS -X POST "$BASE_URL/play_interrupt" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/short-video.mp4","local":true}'
```

Example: A is playing at 125.75 seconds and B → C are waiting. Interrupting with X makes X current and queues A (resume at 125.75s) → B → C. Interrupting X with Y makes Y current and queues X (its saved position) → A → B → C. The interrupted item retains its original playback settings, attribution, and private SMB reference. A generated suggestion becomes permanent when interrupted so switching autoplay off will not discard the saved item. It is queued rather than archived in history at interruption time.

Success returns `202` and `{"ok":true}`. If idle, this acts like play while preserving any waiting queue. With an active item, `409` is returned before pausing if no finite seekable position is reported, a prior browser restore is still pending, or the pause control fails. This includes unsupported browser controls and unseekable/live content. Invalid negative/non-finite start values return `400` on single-item and batch submission.

The original player is closed after pausing; MediaBridge saves the item and position in memory rather than keeping a second player paused in the background. mpv receives an explicit start position with its automatic watch-later resume disabled, including for SMB. For browser playback, MediaBridge waits up to 60 seconds for the page to expose seeking, retrying at one-second intervals. Failure to restore becomes `playback_error`; browser sites may delay or reject seeking. The timestamp is the player's reported snapshot and is not frame-exact. Restarting MediaBridge loses the in-memory queue and its saved positions.

This operation is not transactional across player processes. If closing the old player cannot be confirmed, it returns `409`; if launching the new player fails, it returns `500`. In either case, an already-captured interrupted item remains at the front of the queue for recovery. A later `202` likewise means the new launch was accepted, not that decoding or a future browser seek has completed. While it remains queued, `/status` exposes the saved position as `queue_items[n].start_position_seconds`; launching consumes that marker, so completed history replays normally begin again rather than inheriting an old interruption point.

### Batch additions and literal tail insertion

`POST /queue_batch` accepts `{"mode":"next","items":[{"url":"smb://host/share/a.mp4","local":true}]}`. Use `next` to insert at the front or `last` to append after the entire queue, including suggestions. Selection order is retained. If idle, the first selected item starts. The limit is 1–200 items and a 1 MiB body. All URLs and display indexes are validated before mutation; a player startup failure can still return `500`. Per-item autoplay switches are ignored. Success returns `202` with `{"ok":true,"added":N}`.

The body-size limit is 1,048,576 bytes; invalid/oversized bodies, unsupported modes, empty/over-200 selections, rejected URLs, and invalid display indexes return `400`. This is whole-selection URL/display validation, not an end-to-end playback transaction: SMB credentials, file availability, and other player-startup conditions may fail when an item is actually launched. A `202` means accepted/started, not verified decoding of every selected file. `added` includes the first item if idle playback starts it.

```sh
# Replace current playback, keeping the waiting queue:
curl -sS -X POST "$BASE_URL/play_preserve_queue" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/replay.mp4","local":true}'

# Append after everything, including generated suggestions:
curl -sS -X POST "$BASE_URL/queue" \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/tail.mp4","local":true,"at_end":true}'

# Insert two files next, preserving the selection order:
curl -sS -X POST "$BASE_URL/queue_batch" \
  -H 'Content-Type: application/json' \
  -d '{"mode":"next","items":[{"url":"https://example.com/a.mp4","local":true},{"url":"https://example.com/b.mp4","local":true}]}'
```

With an active current item and a waiting queue `Manual A → Auto X → Auto Y`, each independent operation produces:

| Operation | Waiting queue afterward |
| --- | --- |
| `/queue` Manual B | Manual A → Manual B → Auto X → Auto Y |
| `/queue` Manual B with `at_end:true` | Manual A → Auto X → Auto Y → Manual B |
| `/queue_next` Manual B | Manual B → Manual A → Auto X → Auto Y |
| `/queue_batch` next [B, C] | B → C → Manual A → Auto X → Auto Y |
| `/queue_batch` last [B, C] | Manual A → Auto X → Auto Y → B → C |
| `/play_preserve_queue` B | Manual A → Auto X → Auto Y; B becomes current |
| `/play_interrupt` B | Former current (saved position) → Manual A → Auto X → Auto Y; B becomes current |
| `/play` B | Empty; B becomes current |

`at_end` is specific to `/queue`. `/queue_next` preserves the remaining queue by default and does not require a `preserve_queue` flag.

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

`POST /queue_playlist` inserts all playlist videos before the first generated suggestion in playlist order, preserving existing suggestions. If playback is idle, the first available video starts immediately.

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

`GET /status` returns playback state and the current queue. The `queued` array contains queued URLs; `queue_items` adds metadata and playback options for each waiting entry. See [Playback status response](#playback-status-response).

`POST /queue/remove` removes a waiting entry by its zero-based `index` in the queue. The currently playing item is not part of this index space.

```sh
curl -sS -X POST "$BASE_URL/queue/remove" \
  -H 'Content-Type: application/json' -d '{"index":0}'
```

`POST /queue/play` immediately starts a waiting entry selected by its zero-based `index`. The selected entry is removed from the queue, and the interrupted current item is added to playback history so it can be restored with `/previous`. Returns `404` if the queue index does not exist.

```sh
curl -sS -X POST "$BASE_URL/queue/play" \
  -H 'Content-Type: application/json' -d '{"index":1}'
```

`POST /queue/move` reorders a waiting entry. Both `index` and `to` are zero-based positions in the queue.

```sh
curl -sS -X POST "$BASE_URL/queue/move" \
  -H 'Content-Type: application/json' -d '{"index":2,"to":0}'
```

`POST /queue/promote` turns a generated YouTube Mix entry into a permanent queue entry. Promoted entries stay queued when autoplay is disabled. Optional `queued_by_user_id` assigns attribution to the promoting caller.

```sh
curl -sS -X POST "$BASE_URL/queue/promote" \
  -H 'Content-Type: application/json' -d '{"index":0,"queued_by_user_id":"user-42"}'
```

### Steam on the host

Steam must already be running as the same logged-in desktop user as MediaBridge. These commands act on the MediaBridge PC, independently of Moonlight or Steam Link.

```sh
curl -X POST "$BASE_URL/steam/bigpicture"
curl "$BASE_URL/status"
curl -X POST "$BASE_URL/steam/close"
```

`POST /steam/bigpicture` returns `202` for a background launch request. Poll `steam.phase` and `steam.error` for its outcome. MediaBridge refuses to take over an already-open Big Picture window, an already-running detected game, or a desktop where Big Picture windows cannot be inspected. Repeating Open for an active managed session is a no-op. Steam is not started if it is absent.

Every `/status` response contains a separate `steam` object:

```json
{"active":true,"managed":true,"phase":"game","running":true,"big_picture_open":true,"games":[{"app_id":"1245620","name":"ELDEN RING"}],"error":null}
```

`active` includes a pending launch; `managed` becomes true only after MediaBridge's preflight and open command succeed. Phases are `idle`, `starting`, `browsing`, `game`, `transitioning`, `unknown`, `closing`, `close_failed`, and `error`. IDs are strings (non-Steam shortcut IDs can exceed JavaScript's integer precision); an unresolved name is `null`. All detected games are listed, rather than assuming only one is running. `running` and `big_picture_open` can be `null` when unobserved. An inactive session is not a global Steam monitor.

When no MediaBridge video or Moonlight session is active, the top-level title is `Steam` or `Steam — Game Name`, with a `steam_<phase>` playback state. During concurrent video playback, normal video metadata remains at the top level and Steam information stays under `steam`.

Manual game closure returns the managed session to `browsing` if Big Picture remains. If both the game and Big Picture disappear, the session ends after a three-second grace period (initial opening allows twelve seconds). Steam exiting ends the session. MediaBridge never automatically reopens Big Picture. If inspection fails, the session reports `unknown` instead of claiming it closed. Ownership is in memory: restarting MediaBridge releases it and does not adopt existing Steam sessions.

`POST /steam/close` returns `202` while a managed session closes, or `200` with `closing:false` if there is no managed session. It does not touch video playback or unmanaged Steam sessions. It requests that detected game processes exit, allows three seconds, then force-terminates surviving identified processes; unsaved game progress can be lost. It exits Big Picture and hides/minimizes Steam windows, leaving Steam running. Process identity is checked before termination. Cleanup failures remain visible as `close_failed`; retry Close after resolving them. The regular `/close` action and countdown expiry close video, Moonlight, and managed Steam together; `/steam/close` remains Steam-only.

Game detection uses Steam's bounded process-log history with process creation-time checks; Linux additionally reads same-user Steam app-ID environment markers. Names come from local library manifests and non-Steam shortcut metadata. These are internal Steam formats, so missing logs, unusual launchers, inaccessible processes, or changed formats can prevent detection. Big Picture is identified by Steam-owned windows with known Big Picture titles, including windows covered by a game; it is not inferred solely from the last command. Localized or changed window titles may require additional recognition rules.

Linux requires Python 3 and X11/XWayland window access through `libX11`. Distrobox invokes the helper through `distrobox-host-exec`, so Python, `xdg-open`, and `libX11` must be available on the desktop host. Native Wayland Steam windows that are not exposed through XWayland are not supported. No Steam credentials or Web API key are needed.

### Close countdown

`POST /timer` schedules the same global close action as `POST /close`, after a caller-selected number of minutes. It works independently of autoplay and can be scheduled while idle, during browser/video playback, Moonlight streaming, or a managed Steam session. There is one countdown per MediaBridge process; setting it replaces the previous deadline rather than adding another timer.

```sh
# Close all managed playback in 37.5 minutes:
curl -sS -X POST "$BASE_URL/timer" \
  -H 'Content-Type: application/json' -d '{"minutes":37.5}'

# Replace it with 90 minutes from now:
curl -sS -X POST "$BASE_URL/timer" \
  -H 'Content-Type: application/json' -d '{"minutes":90}'

# Cancel the countdown without stopping anything:
curl -sS -X POST "$BASE_URL/timer" \
  -H 'Content-Type: application/json' -d '{"minutes":null}'
```

`minutes` accepts any positive finite number representable by the host clock, including decimal minutes. There is no preset-only selection or 1440-minute cap. Zero, negatives, nonnumeric values, non-finite values, and unrepresentable deadlines return `400` without changing the active countdown. Explicit `null` or an omitted `minutes` field cancels it. The JSON body limit is 4096 bytes.

Success returns `200` with `{"ok":true,"close_at_epoch_ms":1792700000000}` (illustrative timestamp), or a null deadline when cancelled. Poll `/status` for `close_at_epoch_ms`, `close_remaining_seconds`, and `close_timer_error`. The remaining value is seconds, may be fractional, and is `null` when no countdown is active. Setting or cancelling the countdown clears the previous synchronous timer-cleanup error.

At expiry, MediaBridge consumes the timer once, stops video and controllable browser playback, terminates its Moonlight process group, disables autoplay, cancels background extraction, and clears the waiting queue. It also requests closure of the managed Steam game/Big Picture session even if video was playing simultaneously. History stays in memory. Steam cleanup is asynchronous: inspect `steam.phase`/`steam.error` for completion or later cleanup failure. This uses the existing managed-session rules, including potential loss of unsaved game progress; it does not take over unrelated Steam sessions or arbitrary browser windows outside MediaBridge's controls. The MediaBridge server remains running; this is not OS sleep or host shutdown.

Starting another video, advancing the queue, toggling autoplay, or opening Moonlight/Steam does not reset or cancel the countdown. Manual `/close` clears it. Restarting MediaBridge loses it. Expiry uses a monotonic clock and is checked by the server loop; a blocked player/browser operation or synchronous request can delay cleanup. The dashboard countdown updates through normal status polling. Synchronous close errors appear in `close_timer_error`; every cleanup target is attempted even if another target reports failure. Steam worker errors remain in `steam.error`. A consumed timer is not automatically retried; the user can retry Close or set a new countdown.

Compatibility change: `autoplay_minutes` on single-item/playlist submissions and `duration_minutes` on `/autoplay` now schedule this close action instead of an autoplay-only timeout. `autoplay_minutes` is still ignored for batch items. The old status field `autoplay_until_epoch_ms` remains a deprecated alias of `close_at_epoch_ms`, including when autoplay is off. New clients should use `/timer` and the `close_*` fields.

### Subtitle and audio tracks (mpv)

During local mpv playback, `/status` includes `subtitle_tracks`, `audio_tracks`, `tracks_available`, and `playback_epoch`. Lists come from mpv's live `track-list`, so they refresh as tracks become available and switching Off does not discard them. They are scoped to the current video, not a permanent cross-video archive. Browser playback returns empty lists with `tracks_available:false`; mpv startup/IPC failure can also make the lists temporarily unavailable.

Example status excerpt:

```json
{
  "playback_epoch": 42,
  "tracks_available": true,
  "subtitle_tracks": [
    {"id":1,"language":"eng","title":"English","codec":"subrip","selected":true,"default":true,"forced":false,"external":false,"channels":null},
    {"id":2,"language":"spa","title":"Spanish","codec":"subrip","selected":false,"default":false,"forced":false,"external":true,"channels":null}
  ],
  "audio_tracks": [
    {"id":1,"language":"eng","title":"Original","codec":"aac","selected":true,"default":true,"forced":false,"external":false,"channels":2},
    {"id":2,"language":"spa","title":"Spanish dub","codec":"aac","selected":false,"default":false,"forced":false,"external":false,"channels":2}
  ]
}
```

IDs are mpv track IDs, not array indexes, and are unique only within their type and current playback. Metadata fields can be null when absent. MediaBridge deliberately excludes source URLs and external filenames from this schema, including private SMB relay/credential URLs. `selected` refers to the primary subtitle selection, not a secondary subtitle overlay.

Use `POST /subtitles/select` or `POST /audio/select` to select an ID from its respective list. `id:null` turns that type off; it preserves the loaded list, and another selection can turn it on again without restarting playback. Disabling subtitles also disables secondary subtitles and subtitle visibility. Negative/non-integer IDs are invalid; unknown IDs return `404`. A missing/not-ready mpv session or a mismatched `playback_epoch` returns `409`. Supply the epoch from the same status response as the list to avoid accidentally selecting a track after the video changes.

```sh
# Select subtitle ID 2 and remember its language/title for future playback:
curl -sS -X POST "$BASE_URL/subtitles/select" \
  -H 'Content-Type: application/json' \
  -d '{"id":2,"playback_epoch":42,"remember":true}'

# Turn subtitles off for this video only, retaining its list:
curl -sS -X POST "$BASE_URL/subtitles/select" \
  -H 'Content-Type: application/json' \
  -d '{"id":null,"playback_epoch":42,"remember":false}'

# Change audio language/track:
curl -sS -X POST "$BASE_URL/audio/select" \
  -H 'Content-Type: application/json' \
  -d '{"id":1,"playback_epoch":42,"remember":true}'
```

`remember` defaults to true. A successful switch returns `200` with `{"ok":true,"preference_saved":true,"preference_error":null}`. With `remember:false`, `preference_saved` is false and existing defaults stay unchanged. If the current track switches but saving settings fails, the response remains `200`, with `preference_saved:false` and a readable `preference_error`; clients must display that partial result. Request bodies are capped at 4096 bytes.

Remembered selections use language/title matching, **never a saved numeric track ID**. Exact language/title matches take priority, then language alone; title-only matching is used when no language is known. If no match appears, mpv retains its normal selection. Untagged/untitled tracks cannot identify a specific option in unrelated files. The app also supplies mpv's subtitle/audio language preferences at launch and tries matching newly appearing tracks for up to 60 seconds. Explicit user selection cancels further automatic matching for that type on the current video.

Remembering a subtitle selection enables subtitle loading by default. Remembering subtitle Off sets the saved `subtitles` default to false. An explicit queue item `subtitles:false` always suppresses subtitle loading/selection for that launch, even with a saved language preference. Choosing Off on an already loaded video keeps its list in memory; starting a later video without subtitle loading does not fetch a hidden list for later selection. Embedded subtitles can still be listed, but begin disabled. When loading is enabled, MediaBridge requests all available subtitle languages from yt-dlp and uses the saved preference to select one. Sites can omit languages or reject subtitle access, and this can add extraction work.

Remembering audio Off sets `track_preferences.audio_disabled:true`; selecting an audio track clears it. Audio switching is independent of subtitle loading. A remembered choice applies to future launches and persists in `mediabridge.json` across restarts; already queued items with explicit playback overrides retain those overrides.

### Alternate audio loading and bandwidth

`track_preferences.load_multiple_audio_tracks` defaults to false. A per-item `load_multiple_audio_tracks` overrides it. The dashboard provides both a persistent default and a checkbox for the next submission. Changing the toggle does not reload or prune tracks in an already playing video.

- **Off:** mpv's yt-dlp `all_formats` mode is disabled. For extracted streams, MediaBridge first tries audio formats whose `format_note` identifies them as original, then falls back to the normal audio choice if that metadata is absent. The source may not identify an original track, so this is original/default-only selection, not a guarantee of a particular spoken language.
- **On:** mpv's `all_formats` and `force_all_formats` modes expose alternate extracted formats as selectable tracks. This can expose alternate video formats too, although this API lists only audio/subtitle tracks. mpv normally opens a format when selected rather than streaming all alternatives simultaneously. Switching can be slow or fail on some sources. Selecting a muxed audio/video format may fetch its accompanying video data; selecting audio is not a promise that the original video quality/bandwidth limit will remain the only stream used.
- **Embedded multi-track files/SMB:** these options cannot strip unwanted audio bytes out of an interleaved file or make a single-track source multilingual. mpv reports whatever tracks the file/manifest actually exposes. Disabling alternate-format exposure may reduce probing/requests for some web sources, but a bandwidth reduction is not guaranteed.

These mechanisms follow mpv's documented [track properties](https://mpv.io/manual/stable/#property-list), [track/language selection](https://mpv.io/manual/stable/#track-selection), and [yt-dlp hook options](https://mpv.io/manual/stable/#options-ytdl). Track selection and Off behavior were verified locally with two subtitle and two audio tracks in an isolated Windows mpv fixture. Remote YouTube alternate-audio/subtitle availability and cross-video site behavior remain source-dependent and have not been verified end to end here.

### General playback controls

#### Brief mpv on-screen feedback

Local mpv launches use transient OSD messages rather than a persistent playback-status overlay:

| Trigger | Feedback | Duration |
| --- | --- | --- |
| New video starts | mpv's media title; private SMB playback uses the forced safe title | 2 seconds |
| `/pause`, `/resume`, `/play_pause` | `Paused` or `Playing`, based on mpv's resulting state | 1.2 seconds |
| `/seek`, `/rewind`, `/forward` | mpv's native position/time message and progress bar | 1.2 seconds |
| Volume controls | Current volume percentage | 1.2 seconds |
| `/subtitles/select`, `/audio/select` | Type plus language/title, track ID if unnamed, or `Off` | 1.2 seconds |

The messages are driven by the existing playback/track endpoints; no additional request fields or OSD route is needed. Later messages replace earlier ones instead of accumulating. Track metadata is sent as literal text, stripped of control characters, and capped at 180 characters. Track changes made by initial automatic preference matching do not replace the startup title with extra notifications.

MediaBridge sets `osd-level=1`, `osd-duration=1200`, `osd-on-seek=msg-bar`, and a 2000 ms startup title message. It explicitly requests seek OSD because IPC controls do not always show mpv's normal keyboard feedback by default. OSD delivery is best effort: failure to display feedback does not make an otherwise successful control fail. This applies to local mpv only; browser playback retains the site's own player UI. Existing mpv scripts or manual OSD changes may affect appearance.

The native command prefixes and play/pause/seek/track operations were checked against an isolated Windows mpv instance. That fixture uses null audio/video outputs, so it validates command handling rather than the visual appearance of a particular renderer. See the upstream [mpv OSD options](https://mpv.io/manual/stable/#osd) and [command prefixes](https://mpv.io/manual/stable/#input-command-prefixes).

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
| `/zoom_in` | Increase page zoom in the active managed LibreWolf tab. |
| `/zoom_out` | Decrease page zoom in the active managed LibreWolf tab. |
| `/zoom_reset` | Reset page zoom in the active managed LibreWolf tab to 100%. |

Example:

```sh
curl -sS -X POST "$BASE_URL/pause"
curl -sS -X POST "$BASE_URL/resume"
curl -sS -X POST "$BASE_URL/play_pause"
curl -sS -X POST "$BASE_URL/rewind"
curl -sS -X POST "$BASE_URL/forward"
curl -sS -X POST "$BASE_URL/volume_up"
curl -sS -X POST "$BASE_URL/volume_down"
curl -sS -X POST "$BASE_URL/zoom_in"
curl -sS -X POST "$BASE_URL/zoom_out"
curl -sS -X POST "$BASE_URL/zoom_reset"
```

Zoom commands use LibreWolf's browser-level page zoom and require active playback in the managed LibreWolf tab. Each command changes zoom by one browser step; `/zoom_reset` restores 100%.

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

`POST /close` closes all managed playback together: video/controllable browser playback, Moonlight, and the managed Steam session. It disables autoplay, cancels the close countdown and active background extraction, clears the queue, and preserves session history. Cleanup attempts continue if another target fails. It returns `200` with `{"ok":true,"closing":false}` when no asynchronous Steam close is pending, `202` with `closing:true` when Steam cleanup is pending, or `409` with the synchronous cleanup errors. Poll Steam status for asynchronous completion/errors. The dashboard button is labeled **Close all playback**.

```sh
curl -sS -X POST "$BASE_URL/previous"
curl -sS -X POST "$BASE_URL/next"
curl -sS -X POST "$BASE_URL/close"
```

`POST /sleep` asks the MediaBridge host to enter system sleep. The request is scheduled shortly after the server returns `202` so the response can reach the caller first. It supports Windows and Linux. On Windows, the MediaBridge process needs the system shutdown privilege; on Linux, the session must be allowed to suspend through `systemd-logind`. A separate Wake-on-LAN sender can wake the host after sleep when Wake-on-LAN is enabled in firmware and for the network adapter. MediaBridge leaves wake events enabled but cannot enable Wake-on-LAN on hardware remotely.

```sh
curl -sS -X POST "$BASE_URL/sleep"
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
| `history_items` | Session history, newest first, using the same item schema as `queue_items`. Both include `queued_by_user_id`, `duration_seconds`, and `metadata_error`, without SMB credential references. |
| `hardware_decoder` | mpv decoder name, `software` when disabled, `checking` when unavailable, or `null` for browser playback. |
| `playback_epoch` | Playback generation; send it with track-selection requests to reject stale track IDs. |
| `subtitle_tracks`, `audio_tracks` | Current mpv track lists with IDs, language/title, codec, selection/default/forced/external flags, and channel count where available. |
| `tracks_available` | Whether mpv's track list could be read; false for browser playback or unavailable IPC. |
| `autoplay_loading` | Whether a Mix worker is active. |
| `autoplay_error` | Last operational Mix error, or `null`. |
| `autoplay_status` | Human-readable Mix scan progress; queue positions refer to the full live queue. |
| `playing` | `true` when the state is reported as playing. |
| `mirroring` | Whether a Moonlight process is active. |
| `playback_state` | Player state such as `playing`, `paused`, `idle`, `mirroring`, or `unknown`. |
| `playback_error` | Current playback error, if reported. |
| `title`, `artist`, `album` | Current media metadata. For managed LibreWolf pages, `title` reports the document title when its tab can be matched, including pages without a video. |
| `artwork_url` | Current artwork/thumbnail URL. |
| `channel_avatar_url`, `channel_banner_url` | Channel artwork URLs when available. |
| `media_url` | Active media URL reported by the player, if available. |
| `position_seconds`, `duration_seconds` | Current position and total duration in seconds. |
| `volume` | Volume from 0.0 to 1.0 when available. |
| `can_seek` | Whether a usable duration is available for seeking. |
| `previous_available` | Whether a previous item exists in playback history. |
| `autoplay_enabled` | Whether post-queue autoplay is enabled. |
| `close_at_epoch_ms` | Global close-countdown deadline as Unix epoch milliseconds, or `null`. |
| `close_remaining_seconds` | Remaining countdown seconds (possibly fractional), or `null`. |
| `close_timer_error` | Synchronous expiry-cleanup failure, or `null`. Asynchronous Steam failures are in `steam.error`. |
| `autoplay_until_epoch_ms` | Deprecated alias for `close_at_epoch_ms`; no longer an autoplay-only expiry. |

Example:

```sh
curl -sS "$BASE_URL/status"
```

## Link metadata resolution

### Background metadata and session history

Starting a title-less item schedules metadata; adding title-less queue entries also schedules lookups. Only one metadata worker runs at a time. A current local-mpv lookup waits for playback startup to avoid competing with mpv's extractor, while queued lookups can proceed even if the current browser does not report playback startup.

Successful lookup updates matching current, queue, and history entries, clears their prior `metadata_error`, and retains a positive finite `duration_seconds` when supplied by the resolver. Failed lookup keeps existing metadata and stores a readable error on matching entries. Fields may therefore arrive progressively after a submission; unknown duration is `null`, not zero. There is no queue-total-duration endpoint or durable archive in this API.

`history_items` uses the same public item schema as `queue_items`, with newest entries first. History is held in process memory, capped at 50 entries, and is also used by `/previous`; navigating back can consume an entry. `/close` preserves it, but restarting MediaBridge loses it. The current item remains represented separately by the top-level status fields.

Example public queue/history item (illustrative; the same nullable metadata fields can be absent from the extractor):

```json
{
  "url": "smb://fileserver/media/movie.mp4",
  "queued_by_user_id": "user-42",
  "duration_seconds": null,
  "title": "movie.mp4",
  "metadata_error": null,
  "uploader": null,
  "thumbnail_url": null,
  "channel_avatar_url": null,
  "channel_banner_url": null,
  "local": true,
  "browser": null,
  "subtitles": null,
  "load_multiple_audio_tracks": null,
  "start_position_seconds": null,
  "quality": null,
  "screen": null,
  "autoplay_generated": false
}
```

This is the complete public item shape; it does not serialize every request option. In particular, it omits `smb_share_id`, credentials, private relay URLs, browser arguments, and per-item autoplay switches. The top-level `duration_seconds` describes current playback; an entry's `duration_seconds` describes that queued/history item.

Selected current status fields during a Mix lookup could look like this (partial object):

```json
{
  "autoplay_enabled": true,
  "autoplay_loading": true,
  "autoplay_error": null,
  "autoplay_status": "Checking current video · 2 queued candidates (max 20)",
  "hardware_decoder": "nvdec-copy",
  "playback_error": null
}
```

`hardware_decoder` reads mpv's `hwdec-current`: `no` becomes `software`, an unavailable property becomes `checking`, and browser/non-mpv status returns `null`. A decoder name describes active decoding; it does not measure encoding performance or prove a particular streaming configuration works.

### Explicit resolution requests

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

`GET /preferences` returns saved playback defaults: `default_screen`, `default_quality`, `default_video_codec`, `subtitles`, `preferred_language`, `track_preferences`, `cookies_browser`, and `cookies_profile`.

`POST /settings/preferences` saves those defaults. This replaces the preference values rather than patching selected fields, so send the full desired settings object. Omitted fields use the defaults in the table below.

| Field | Default for an omitted request field |
| --- | --- |
| `default_screen` | `null` |
| `default_quality` | `"720"`; explicit `null` also becomes `"720"` |
| `default_video_codec` | `"h264"`; explicit `null` is invalid |
| `subtitles` | `false` |
| `preferred_language` | `null` |
| `track_preferences` | `{"subtitle":null,"audio":null,"audio_disabled":false,"load_multiple_audio_tracks":false}` |
| `cookies_browser` | `"auto"`; an existing installed configuration can have a platform-specific browser selection |
| `cookies_profile` | `null` |

```sh
curl -sS "$BASE_URL/preferences"
curl -sS -X POST "$BASE_URL/settings/preferences" \
  -H 'Content-Type: application/json' \
  -d '{"default_screen":1,"default_quality":"1080","default_video_codec":"h264","subtitles":true,"preferred_language":"en","cookies_browser":"librewolf","cookies_profile":null}'
```

`default_screen` is `null` or 0–32. `default_quality` accepts `"best"` or a numeric resolution from 144–4320; omitted/null requests fall back to `"720"`. Configuration loading also migrates missing/null or saved `"best"` defaults to `"720"`. Explicit per-item `quality:"best"` remains uncapped. `default_video_codec` is `h264` (default), `vp9`, or `auto`, with fallbacks when the preferred codec is unavailable. The dashboard offers 2160, 1440, 1080, 720, and 480 presets. `cookies_browser` can be `auto`, `off`, `librewolf`, `chrome`, `chromium`, `edge`, `firefox`, `brave`, `opera`, `vivaldi`, or `whale`. Selected cookies also apply to background YouTube metadata, playlists, and Mix extraction. A cookie profile is optional and must not contain commas or line breaks.

For local mpv extraction, codec ordering is H.264 → VP9 → general video/audio → combined format when `h264` is selected, and VP9 → H.264 → the same general fallbacks for `vp9`. `auto` uses general video/audio and combined fallbacks. Every fallback applies the requested height cap; `quality:"best"` removes that cap without overriding the codec preference. MediaBridge always passes its constructed format selector to mpv. These settings select available streams; they do not transcode media or alter browser playback quality.

A saved `default_quality:"best"` can apply to the live preferences after saving, but configuration loading on restart migrates it to 720p. The dashboard also normalizes missing/unsupported quality values to 720p and does not offer an uncapped choice; the Rust API still accepts numeric heights throughout 144–4320 and explicit per-item `best`.

Background cookie inheritance applies only to YouTube URLs and respects `cookies_browser:"off"`. It uses the configured browser/profile, including the managed LibreWolf profile via yt-dlp's Firefox reader. Preference-selection errors are propagated; the worker does not silently choose another identity. Logs report cookie use without cookie contents. This does not cause unrelated external processes or an integration's own metadata resolver to inherit these settings.

`GET /platform` returns `os`, `flatpak` (boolean), and `steam_host_access_configured` (boolean or null). The last field reads the effective session-bus policy of the running Flatpak sandbox; it is null outside Flatpak or when that policy cannot be read. True means host-execution permission is configured, not that host Python/libX11 or Steam detection works. False means the permission is absent. Permission changes require quitting and reopening the Flatpak; rechecking before restart still reports the old session. The dashboard's Flatpak-only **Settings → Steam host access** section shows the state and setup command. The API does not grant or revoke sandbox permissions.

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

The `track_preferences` object stores optional `subtitle` and `audio` choices, each shaped as `{"language":"eng","title":"Original"}` (either field may be null), plus `audio_disabled` and `load_multiple_audio_tracks` booleans. Language tags must contain only ASCII letters, digits, and hyphens, up to 64 bytes; remembered titles are capped at 512 bytes without control characters. Full settings updates must include existing `track_preferences` to preserve them, since settings replacement is not a partial patch. The dashboard carries them forward automatically and clears a saved subtitle match when the manually entered preferred subtitle language changes.

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

The Now playing card uses an icon toolbar: previous, back five seconds, play/pause, forward five seconds, next, and close all playback. The play/pause button follows the reported player state and uses the existing pause/resume/toggle commands. Icons have accessible names and tooltips, with 44-pixel touch targets (40 pixels on very narrow screens). No endpoint semantics change.

The dashboard uses a compact responsive layout. Playback controls stay prominent; **Playback options** (including attribution/SMB access), **More play actions**, **Add a batch**, **History**, **Steam session**, and **Audio & subtitles** expand to reveal their controls. **Settings** contains grouped Accounts, Network access, Playback defaults (including Browser cookies), and Server port settings. Question-mark help controls expose detailed guidance by tap/click or keyboard activation, with hover/focus styling on desktop. Source URLs remain available through per-item help. Shorter labels do not change the endpoints below: **Queue** inserts before Mix suggestions, and **Queue at end** appends after everything. Every previously available control remains available; no server setting or API option was removed. The playback-mode selector stays visible in Sunshine mode so users can switch back without reloading.

The dashboard exposes the API improvements as follows:

| Control/display | API behavior |
| --- | --- |
| Play now | `/play`, or `/play_playlist` for a playlist; replaces the queue. |
| Play now · keep queue | `/play_preserve_queue` for one video. |
| Play now · resume current next | `/play_interrupt` for one video; disabled when a current item reports no seeking. Server checks still apply. |
| Play next / Play last | Existing queue/playlist routes. Ordinary Play last inserts before Mix suggestions. |
| Append after everything | `/queue` with `at_end:true` for one video. |
| Add a batch | One URL per line, 1–200 items, `/queue_batch` with next/last. Uses the form's playback settings. Each URL is an individual item; playlists are not expanded. |
| Session history | `history_items`, newest first. Replay now keeps the queue; next/last enqueue a copy. Selected history entries are submitted in displayed order through `/queue_batch`. Selection clears if history URL ordering changes. |
| Submitter ID | Optional caller-supplied `queued_by_user_id` on new submissions, history replays, and Mix promotion. No login or name lookup is added. |
| Queue/history metadata | Duration (or explicitly unknown), submitter ID, lookup errors, and saved resume position are shown. History responses now also include `browser` and `subtitles` so replay can retain those settings. |
| Decoder and Mix status | `hardware_decoder`, `autoplay_status`, `autoplay_loading`, and `autoplay_error`. Dismiss Mix error calls `/autoplay/error/clear` without resetting cooldown. |
| Subtitle/audio selectors | Live mpv track lists, Off controls, and a checked-by-default remember switch. `/subtitles/select` and `/audio/select` carry the playback epoch. |
| Alternate audio toggle | Per-video and persistent `load_multiple_audio_tracks` controls; off requests original/default extracted audio only. |
| Close countdown | Custom numeric minutes, remaining time, and start/replace/cancel controls using `/timer`. Works independently of the autoplay toggle. Expiry and Close all playback use the same global cleanup. |
| SMB provider reference | Optional manually entered 24-hex `smb_share_id` applied only to SMB URLs. This is a provider reference, not a password prompt or share browser. |

Public history omits private SMB references. To replay provider-backed SMB, enter the matching reference in the form first; a multi-selection must use files authorized by that reference. Leaving it blank requests direct guest SMB. The dashboard does not persist the provider reference or submitter field in browser storage. History selection is also held only in memory. No share-browsing, credential-saving service, or user authentication is added by these controls.

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
| `501` | The requested operation is unsupported on this platform. |

Error bodies generally use this shape:

```json
{"error":"Human-readable explanation"}
```
