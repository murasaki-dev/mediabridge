# MediaBridge

MediaBridge is a small LAN-controlled process for sending web video to a TV-connected Linux PC. Its application logic lives in one Rust file. Installers bundle Moonlight for Sunshine streaming. On Linux, the installer also supplies private LibreWolf and Moonlight AppImages for dashboard, browser playback, and game streaming.

The separate phone-friendly discovery and remote app is in [`discovery/`](discovery/README.md). It can be run with Docker Compose without changing the MediaBridge player build.

## Build and run

On Windows, run [`install-deps.bat`](install-deps.bat) to install Rust and the Visual C++ build tools, then build the release executable. The Windows build starts in the notification area without opening a console. Left-click the MediaBridge tray icon to open a live log console; right-click it for **Show logs** and **Quit MediaBridge**. Logs are also saved at `%LOCALAPPDATA%\MediaBridge\mediabridge.log`.

To create a self-contained Windows setup program, run [`build-installer.bat`](build-installer.bat). It rebuilds MediaBridge from the current workspace first, then installs 7-Zip and Inno Setup with winget if needed, downloads the standard x86_64 mpv build, yt-dlp, Deno, the official LibreWolf portable browser, and the official Moonlight x64 portable client, and packages them into `dist\MediaBridge-Setup.exe`. The installer version is read from `Cargo.toml`. It asks which TCP port MediaBridge should use (8765 by default), passes it to the app, and creates a matching private-network firewall rule. The app and runtime tools are installed side by side in `C:\Program Files\MediaBridge`; the generated `unins000.exe` is in that same folder. The post-install action opens the onboarding dashboard in LibreWolf. Starting MediaBridge automatically at Windows sign-in is enabled by default and can be disabled in the installer; it runs in the interactive user session so the tray and browser controls work. Uninstalling stops MediaBridge and removes its startup shortcut, firewall rules, installed files, and bundled LibreWolf profile. Choosing to remove user data also deletes settings and logs under `%LOCALAPPDATA%\MediaBridge`. Windows may ask for administrator approval to install the app and manage its firewall rule. The setup executable is self-contained once built; network access is needed when building it to download the current tool releases.

On supported x86_64 glibc Linux distributions, run the installer as your normal desktop user:

```sh
curl -fsSL https://raw.githubusercontent.com/murasaki-dev/mediabridge/main/install-linux.sh | bash
```

The single installer detects apt, dnf, pacman, zypper, or xbps; installs MediaBridge's runtime dependencies; downloads the latest LibreWolf AppImage and Moonlight AppImage from their official sources; and installs them with MediaBridge under the current user's home directory. The AppImages are extracted for systems that do not provide AppImage/FUSE support. It creates a private persistent LibreWolf profile, enables cookie retention for that profile, and enables the MediaBridge user service at login. When run in a desktop session, it opens the onboarding dashboard in LibreWolf. On desktops with a StatusNotifier tray host, the tray menu opens the dashboard in LibreWolf, opens the log file, or quits MediaBridge. Other desktops can still use the application-menu launcher. The dashboard is available at `http://127.0.0.1:8765/`.

To remove the Linux app, run `mediabridge-uninstall`. This stops and removes the user service, launchers, and desktop entries while preserving settings, logs, and browser cookies for a possible reinstall. Use `mediabridge-uninstall --purge-data` to also delete those personal files. System dependencies stay installed because other programs may use them.

There is no GitHub Release yet, so the download command will work after publishing the `mediabridge-linux-x86_64.tar.gz` asset. To build and install from a Linux checkout first:

```sh
chmod +x build-linux.sh install-linux.sh
./build-linux.sh
./install-linux.sh --local
```

The build script creates an x86_64 Linux tarball, not a `.deb`. `--no-autostart` installs without enabling the login service. `--deps-only` installs runtime dependencies without installing MediaBridge.

For a foreground source run, install Rust/Cargo separately, then install runtime dependencies with `./install-linux.sh --deps-only`. Build and run with the development runner:

```sh
chmod +x install-linux.sh run.sh
./install-linux.sh --deps-only
```

Then build and launch the server in the foreground with the existing development runner:

```sh
./run.sh
```

`build-linux.sh` is the developer-only release builder and requires Rust/Cargo, `pkg-config`, GTK 3 and AppIndicator development libraries on Linux x86_64. End users installing from a GitHub Release do not need Rust or a compiler. The current portable release target is x86_64 glibc Linux; Alpine/musl and ARM need separate build targets.

Local mpv playback uses the persistent MediaBridge LibreWolf profile on Linux and Windows. The first dashboard visit shows **Connect your accounts** at the top; dismiss it when finished, and reopen it later from **Settings**. Use its sign-in buttons, then **Check** to confirm each site's cookies can be read. YouTube checks validate yt-dlp access to a video and try to show the active channel name and avatar from YouTube's account menu, without OAuth or a Google developer app. Twitch checks confirm that yt-dlp can read the browser and that a Twitch authentication cookie is present; they do not yet validate that session or resolve Twitch profile artwork. Cookie data is read locally and the temporary cookie export is deleted when the check finishes; cookie contents are never included in MediaBridge API responses. The installer provides Python SecretStorage support for Linux cookie access.

By default, MediaBridge listens on all network interfaces at port 8765. Change the port from **Settings → Server port → Apply and restart**. The dashboard follows the server to its new port. Select **Sunshine / Moonlight** in the Add media **Playback** menu to pair with or connect to a host. Pairing opens Moonlight's PIN screen; enter that PIN in Sunshine, then close Moonlight's success dialog so MediaBridge can confirm the pairing. The Windows installer opens its selected install port in Windows Firewall; after changing it, update the **MediaBridge Server** rule to the new TCP port. Linux setup opens TCP 8765 in active UFW or firewalld configurations; after changing it, add the new port there. The installers do not turn on inactive firewalls or modify unknown firewall managers. The router and network must also allow local devices to reach the PC.

Open `http://<mediabridge-ip>:8765/` on a device on the same network to use the lightweight testing dashboard. It lets you choose browser or local mpv playback, paste and preview links, set a local playback quality cap, manage the queue, and control the current stream. YouTube playlist links are expanded into their available videos in playlist order: **Play now** starts the first video and queues the rest, while **Play last** and **Play next** add the playlist videos to the queue. Queue entries show their resolved title/uploader when yt-dlp can identify them, and can be moved or deleted. The page is embedded into the program at build time.

Moonlight is bundled in the Windows and Linux installers. MediaBridge prefers the bundled client for Sunshine streaming and pairing. The packaged Windows build uses Moonlight's normal per-user settings store, so the dashboard-launched client and a normal Moonlight Qt installation use the same pairing identity rather than separate portable-folder settings. The pairing card is remembered by the dashboard browser after Moonlight confirms success. You can select another Moonlight executable with `--moonlight PATH` or `MEDIABRIDGE_MOONLIGHT`; the packaged client is the official [Moonlight Qt](https://github.com/moonlight-stream/moonlight-qt) project. Remote mouse commands use `xdotool` on X11. On Wayland they use `ydotool`, which requires a running `ydotoold` process with permission to access `/dev/uinput`. When the distribution provides a system `ydotool.service`, the installer configures a shared socket and adds the desktop user to the `mediabridge-input` group; log out and back in for the group change to take effect.

Browser playback uses the app-managed LibreWolf profile on Linux and Windows. MediaBridge uses Firefox WebDriver BiDi over LibreWolf's loopback-only remote debugging interface to read the active video's state, control playback and seek/volume, enter fullscreen mode, close LibreWolf from **Quit**, and advance through the queue when a video ends. YouTube playlist playback starts the first listed video and queues the rest. yt-dlp is not run to resolve ordinary browser-mode links; it is still used when expanding a YouTube playlist, previewing a link, checking cookies, and by mpv. Alternatively set `local:true` to play supported links directly with mpv; mpv uses the installed `yt-dlp` integration for YouTube, Twitch, and other supported sites. This avoids browser UI automation and uses mpv's local IPC for playback controls and live status. Set `quality` to `"720"`, `"1080"`, or another height from 144 to 4320 to cap the selected stream resolution; omit it or use `"best"` for mpv's default best-available selection. To choose a display, set `screen` to a zero-based display index from 0 to 32. This works with local mpv and, on Windows, with managed LibreWolf playback; omit it to let the player or desktop choose. The dashboard has an optional **mpv display index** field. Display selection is platform and window-system dependent; Wayland compositors control window placement, so they may not honor a requested screen. Quality is chosen at start and is not dynamically changed during playback yet. The browser path remains available for sites that require browser login or do not work through yt-dlp.

YouTube's old TV/Leanback web interface is not an officially supported desktop experience. It may still load when Chromium uses a TV user-agent. To try the experimental path with a video link, edit and run `test-youtube-tv.bat`; the script opens `youtube.com/tv#/watch?v=VIDEO_ID` in Chromium's app window and sets `youtube_guest:true`. When enabled, MediaBridge waits for a visible control labeled exactly “Watch as guest” and clicks its verified on-screen position. It does not bypass sign-in or infer a URL parameter; it only selects YouTube's own guest option if the TV page presents it. If the control is missing or the page is not YouTube TV, it sends no click. The TV interface may change or stop working without notice.

Twitch does not document a theater-mode URL parameter. For Chromium requests to a Twitch URL, set `twitch_theater:true` to have MediaBridge wait for the stream player and send Twitch's `Alt+T` theater-mode shortcut. The updated `test-twitch.bat` opts in.

YouTube requests default to regular YouTube. Set `youtube_mode:"tv"` to open the experimental TV/Leanback interface; MediaBridge converts a YouTube video URL to the matching TV URL and supplies the TV browser flags. The updated `test-youtube-tv.bat` tests this option. Regular YouTube mode uses the documented `F` fullscreen shortcut first, then falls back to the visible fullscreen control if needed. See `test-youtube.bat` for a regular-mode test.

The default Moonlight command is `moonlight stream HOST Desktop`. Override the executable with `--moonlight PATH` or `MEDIABRIDGE_MOONLIGHT`, and override the arguments with repeated `--moonlight-arg` options and `{host}` / `{app}` placeholders. These templates allow the target's installed Moonlight version and desired app name to be configured without changing the program.

## Configuration

On Linux, MediaBridge reads `~/.config/mediabridge/mediabridge.json` (or `$XDG_CONFIG_HOME/mediabridge/mediabridge.json`). It migrates an existing configuration beside the executable the first time the new package starts. On Windows, the installer creates `mediabridge.json` beside the executable, grants the installing Windows account permission to update it, and preserves it during upgrades. Open **Settings** in the dashboard to manage allowed remote origins and persistent playback defaults. These changes save to the config file and apply to future playback immediately. If MediaBridge cannot create or save the file, the dashboard reports the error and the existing settings remain active.

Example:

```json
{
  "bind": "0.0.0.0:8765",
  "remote_origin_servers": ["https://party.example"],
  "default_screen": null,
  "default_quality": null,
  "subtitles": false,
  "preferred_language": null
}
```

`remote_origin_servers` is a list of exact web origins allowed to call MediaBridge from a browser, including scheme and optional port, with no path (for example `https://party.example` or `http://192.168.1.10:8080`). Add each self-hosted web app's origin to each MediaBridge installation that should accept it. These entries configure browser cross-origin access; they do not make MediaBridge connect to those servers. Requests carrying a different web origin are rejected. Same-origin dashboard use and command-line clients without an `Origin` header continue to work. Browser support for reaching local-network services may also require a permission prompt. The API still has no user authentication, so only allow web apps and users you trust.

`bind` sets the listen address and port when no `--bind` command-line argument overrides it. `default_screen` uses a zero-based display index (0–32) for mpv and, on Windows, managed LibreWolf video windows, `default_quality` accepts `best` or a height from 144 to 4320, `subtitles` controls whether mpv requests subtitles by default, and `preferred_language` sets yt-dlp's preferred subtitle language when subtitles are enabled. The dashboard loads the display, quality, and subtitle defaults on startup. Per-request playback values override these defaults where that request field is supplied.

## HTTP API

For copyable `curl` commands and details for integrating with the HTTP endpoints, see the standalone [MediaBridge HTTP API guide](API.md).

Requests use JSON. No authentication is enabled; control is available to any device that can reach the listening port. Browser playback controls and metadata use Linux MPRIS over the user's desktop D-Bus session; local mpv playback uses a private Unix IPC socket. Run the service as the logged-in desktop user. Both modes work on X11 and Wayland.

| Method | Path | Body | Behavior |
|---|---|---|---|
| GET | `/` | — | Testing dashboard |
| GET | `/status` | — | Queue, playback state/error, media metadata, position, duration, artwork URL, and volume |
| GET | `/preferences` | — | Public playback defaults used to initialize the dashboard |
| GET | `/settings/remote-origins` | — | Read the configured browser-origin allowlist |
| POST | `/settings/remote-origins` | `{"remote_origin_servers":["https://party.example"]}` | Replace and save the browser-origin allowlist |
| POST | `/settings/preferences` | `{"default_screen":1,"default_quality":"1080","subtitles":true,"preferred_language":"en","cookies_browser":"auto","cookies_profile":null}` | Save persistent playback defaults, including browser-cookie selection |
| POST | `/settings/cookies/check` | `{"cookies_browser":"auto","cookies_profile":null}` | Verify local browser-cookie access and resolve a YouTube video |
| GET | `/settings/port` | — | Read the current TCP port |
| POST | `/settings/port` | `{"port":9000}` | Save a new port, then restart the server and rebind |
| POST | `/resolve` | `{"url":"https://…"}` | Resolve a pasted link with yt-dlp and return canonical URL, title/uploader, thumbnail, and optional channel artwork when supported |
| POST | `/play` | `{"url":"https://…","local":true,"quality":"720"}` | Replace current playback with mpv + yt-dlp; omit `quality` for best available |
| POST | `/play` | `{"url":"https://…","local":true,"screen":1}` | Open local mpv fullscreen on display index 1 (zero-based; valid range 0–32) |
| POST | `/play` | `{"url":"https://…","browser":"chromium","youtube_mode":"regular","youtube_guest":true,"twitch_theater":true}` | Browser playback; browser is optional. YouTube mode defaults to `regular`; `tv` selects Leanback |
| POST | `/queue` | `{"url":"https://…","local":true,"quality":"1080"}` | Queue a local mpv item; the queue can contain both browser and local items |
| POST | `/queue` | `{"url":"https://…","browser":"/usr/bin/firefox","browser_args":["--kiosk","{url}"]}` | Queue browser playback; start immediately if idle |
| POST | `/queue_next` | Same body as `/play` | Insert this item at the front of the queue |
| POST | `/queue/remove` | `{"index":1}` | Delete a waiting queue item by its zero-based queue position |
| POST | `/queue/move` | `{"index":2,"to":0}` | Reorder a waiting queue item |
| POST | `/close` | — | Stop playback/mirroring and clear the queue |
| POST | `/pause` | — | Pause playback |
| POST | `/resume` | — | Resume playback |
| POST | `/play_pause` | — | Toggle pause/play |
| POST | `/rewind` | — | Seek backward 5 seconds |
| POST | `/forward` | — | Seek forward 5 seconds |
| POST | `/seek` | `{"position":123.5}` | Seek to an absolute position in seconds |
| POST | `/volume_up` | — | Increase media volume by 0.05 |
| POST | `/volume_down` | — | Decrease media volume by 0.05 |
| POST | `/volume` | `{"volume":0.5}` | Set volume from 0.0 to 1.0 |
| POST | `/mouse` | `{"x":640,"y":360,"action":"click","button":"left"}` | Absolute pointer move/click; `move_relative` accepts `dx`/`dy`, and `click_current` clicks the current pointer position |
| POST | `/next` | — | Stop current playback and start the next queued URL |
| POST | `/previous` | — | Return to the previous item and put the interrupted item at the front of the queue |
| POST | `/sunshine` | `{"host":"host-or-ip","app":"Desktop"}` | Stop playback and launch Moonlight for that host/app (app defaults to `Desktop`) |
| POST | `/sunshine/pair` | `{"host":"host-or-ip"}` | Open Moonlight's pairing PIN screen for the Sunshine host |
| GET | `/sunshine/pair/status` | — | Check whether the active Moonlight pairing completed |

Example (HTTP media):

```sh
curl -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=…","browser":"chromium"}' http://tv-pc:8765/queue
```

For an SMB share, send `{"url":"smb://fileserver/media/movie.mp4","local":true}` to `/play` or `/queue`. MediaBridge passes the SMB URL directly to mpv with yt-dlp disabled. The share must allow guest access; the app never prompts for credentials and rejects usernames/passwords embedded in an SMB URL. Use `smb://server/share/path/to/file.mp4` (URL-encode spaces). mpv needs FFmpeg SMB support. Browser playback and `/resolve` do not accept SMB URLs.

On the MediaBridge machine, test playback and queueing with a public YouTube URL:

```sh
curl -X POST http://127.0.0.1:8765/play -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ","browser":"chromium"}'
curl -X POST http://127.0.0.1:8765/queue -H 'Content-Type: application/json' \
  -d '{"url":"https://www.youtube.com/watch?v=jfKfPfyJRdk"}'
curl http://127.0.0.1:8765/status
```

Stop playback with `curl -X POST http://127.0.0.1:8765/close`.

To move or click the Linux host's pointer from another LAN device, send `POST /mouse` with `x` and `y` in screen pixels and `action` set to `move` or `click` (button defaults to `left`). `move_relative` accepts signed `dx`/`dy`; `click_current` clicks without moving. Buttons are `left`, `right`, or `middle`. The dashboard's Host touchpad uses relative movement, so it does not depend on the host's display resolution. Edit and run `test-mouse.bat` on Windows; set `ACTION=move` first to check coordinates without clicking. For example, from PowerShell:

```powershell
Invoke-RestMethod -Method Post -Uri http://192.168.0.200:8765/mouse -ContentType 'application/json' -Body '{"x":960,"y":900,"action":"click","button":"left"}'
```

Adjust the coordinates to the host display resolution and the location of the target. This can be used to click YouTube TV's guest option when its layout is stable; MediaBridge does not guess the coordinates.

`GET /status` also returns `playback_state`, `playback_error`, `title`, `artist`, `album`, `artwork_url`, `channel_avatar_url`, `channel_banner_url`, `media_url`, `position_seconds`, `duration_seconds`, `volume`, and `can_seek`. For local mpv playback, MediaBridge detects YouTube bot-verification prompts and HTTP 403 stream errors from mpv's log, prints a readable warning, and exposes it as `playback_error` so the dashboard and future Home Assistant integration can explain why playback failed. It also attempts a native desktop notification through `notify-send`; this uses the freedesktop notification standard over the desktop session bus, supported by common X11 and Wayland desktops. A notification daemon must be active in that session, so this is best-effort; missing notification support never affects playback or queue advancement. The warning remains visible if the queue advances and clears when playback is explicitly replaced or closed. The dashboard shows the current video thumbnail, plus channel avatar/banner when the media extractor provides them. `yt-dlp` usually provides video thumbnails, but channel artwork is extractor-dependent and may be absent. Those metadata fields are refreshed when `/status` is polled, so a future Home Assistant integration can poll it for a live media card and use `/seek` for its progress slider. Browser playback can also provide MPRIS artwork when the browser publishes it.

For media controls, use `curl -X POST http://127.0.0.1:8765/pause`, `/resume`, `/play_pause`, `/rewind`, `/forward`, `/volume_up`, `/volume_down`, or `/next`. To seek to an exact time or set an exact volume:

```sh
curl -X POST http://127.0.0.1:8765/forward
curl -X POST http://127.0.0.1:8765/volume_down
curl -X POST http://127.0.0.1:8765/seek -H 'Content-Type: application/json' -d '{"position":125}'
curl -X POST http://127.0.0.1:8765/volume -H 'Content-Type: application/json' -d '{"volume":0.5}'
curl -X POST http://127.0.0.1:8765/next
```

To start a paired Sunshine host (`192.168.1.50` here), install Moonlight Qt and run:

```sh
curl -X POST http://127.0.0.1:8765/sunshine -H 'Content-Type: application/json' \
  -d '{"host":"192.168.1.50","app":"Desktop"}'
```

Replace the sample host with the Sunshine PC's LAN IP or hostname.

## Current scope

This is an initial prototype. It delegates website playback to a full browser, so playback depends on the site's browser support and may show browser-specific prompts or errors. On Linux, the app-managed LibreWolf AppImage is built for x86_64 glibc systems; other architectures and musl distributions need separate builds. A tray icon appears only when the desktop provides a compatible AppIndicator host or extension; the dashboard and autostart launchers remain available when it does not. The dashboard can start and verify Moonlight pairing, but it does not check Sunshine availability before connecting. BiDi controls and status are enabled for the managed LibreWolf profile; other browsers depend on MPRIS and may provide limited metadata or controls. The Home Assistant integration can poll `/status` for current position and duration, then send `/seek` updates from its progress slider.
