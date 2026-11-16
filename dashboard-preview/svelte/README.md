# MediaBridge dashboard preview

Static SvelteKit dashboard for MediaBridge. Release builders embed its compiled files in the native executable. For previewing changes, the Docker container serves the same dashboard and forwards its API requests to a MediaBridge host.

## Run on this PC

From the workspace root:

```powershell
docker build -t mediabridge-dashboard-preview:local dashboard-preview/svelte
docker run -d --name mediabridge-dashboard-preview -p 127.0.0.1:18765:8080 -v mediabridge-dashboard-appearance:/app/data -e MEDIABRIDGE_UPSTREAM=http://192.168.0.14:8765 mediabridge-dashboard-preview:local
```

Open http://127.0.0.1:18765. To replace an existing preview container, remove the old preview with `docker rm -f mediabridge-dashboard-preview` before running the second command. Rebuild after editing the Svelte source.

## Checks

From this directory, with Node 24:

```sh
npm ci
npm run check
npm test
npm run build
mkdir -p data
node serve.mjs
```

The production output is in `build/`. The preview serves its SPA routes at `/search`, `/chat`, `/mousepad`, `/appearance` and `/settings`. The local preview proxy defaults to port 8080.

## API contract

- `GET /status` supplies the current media, transport state, seek support, queue, and history. Queue and history come from this snapshot. The dashboard paginates and virtualizes them locally.
- `GET /recommendations/home` and `GET /recommendations` supply YouTube Home and current-video recommendations, respectively. Home requires the API-enabled build. Personalized results depend on the Obscura profile on the host.
- `POST /play_preserve_queue`, `/queue`, `/pause`, `/resume`, `/next`, `/close`, `/seek`, and `/autoplay` control host playback. `POST /sunshine` launches Moonlight for an already paired bookmark.
- Browser playback on `/watch` takes direct HTTP(S) media and optional WebVTT URLs. It uses native HLS where available and lazy-loads HLS.js otherwise. The media origin must permit CORS. YouTube links use host playback.
- Theme, bookmarks, and resume positions live in the browser. The preview has no dependency on server-side rendering or a server-side preferences store.

Targets current Chrome, Edge, Firefox, and Safari with CSS Grid, dynamic viewport units, and Web Storage. Space toggles playback and the arrows seek ten seconds when focus is outside interactive controls.

**Search** runs Obscura on demand with a temporary signed-out profile. The Docker preview uses a pinned Obscura binary and native builds use the host’s Obscura. Results are cached for two minutes. **Chat** follows playback on the MediaBridge host and offers a direct chat link. MediaBridge checks YouTube’s live chat document through Obscura while the Chat page is open. Twitch channel playback opens its chat directly. Open the preview with `localhost` as its hostname for Twitch. Provider sign-in in the browser controls chat participation.

The top playback dock expands into the current queue. Queue actions use MediaBridge’s positional queue API. Server settings are loaded from the host’s Simple dashboard within Settings so that both interfaces expose the same host controls. YouTube cookie import remains restricted to the dashboard running directly on the MediaBridge host.

Appearance is available at /appearance. The currently uploaded wallpaper is shared by preview clients and persisted in the Docker volume. Theme, panel and wallpaper opacity, animation duration, and sidebar width are browser-local.

The preview defaults to Simple. Open **Settings → Dashboard mode** and select **Fancy** to enable recommendations, subscriptions, history, and Appearance. The browser saves the choice. Simple skips those feed requests, pauses status polling while hidden, and reduces polling when playback is idle. The server's scheduled refresh setting is independent of the dashboard mode.

Subscriptions require a MediaBridge server build with the subscriptions endpoints. Sign in through managed LibreWolf on the host and use **Accounts → Import cookies** on the host's loopback dashboard. The button refreshes both YouTube and Twitch in their separate Obscura profiles.
