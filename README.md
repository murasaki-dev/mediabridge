# MediaBridge

Control media on a Windows or Linux PC from another device on your LAN. MediaBridge provides a browser dashboard and HTTP API for browser video, local mpv playback, and Sunshine/Moonlight streaming.

## What it does

- Play web links in a managed LibreWolf browser or directly with mpv and yt-dlp.
- Queue videos and YouTube playlists, reorder entries, replay history, and add batches without losing queued suggestions.
- Interrupt with a new video, then resume the previous one next from its saved timestamp.
- Set a custom close countdown to stop media, browser playback, Moonlight, and managed Steam together, independently of autoplay.
- Continue with YouTube Mix suggestions, with visible lookup progress and errors.
- Control playback, seeking, volume, browser zoom, and the host pointer.
- Switch mpv subtitle/audio tracks, remember language choices, or disable them; alternate audio loading is optional.
- Reuse local browser sign-ins for playback and background YouTube lookups.
- Pair Moonlight with a Sunshine host and launch desktop/game streaming.
- Open a managed Steam Big Picture session on the host, report running games, and close it while leaving Steam running. Video playback can run alongside Steam; manually opened Steam sessions stay unmanaged.

Local playback starts with **720p and H.264 preferences**, with codec fallbacks and configurable hardware decoding. SMB guest playback requires an SMB-enabled mpv build; authenticated SMB or relay playback requires an external provider. Standalone MediaBridge does not include an SMB login store or file browser. See the [SMB API guide](API.md#smb-credential-providers).

## Install

### Linux — Flatpak recommended

Requires Flatpak on x86_64 Linux. Download these four files from [Releases](https://github.com/murasaki-dev/mediabridge/releases) into the same folder:

- `MediaBridge.flatpak`
- `MediaBridge-Platform.flatpak`
- `install-flatpak.sh`
- `start-flatpak.sh`

Then run from that folder, without sudo:

```sh
bash install-flatpak.sh
```

The installer adds the app and its isolated runtime, then opens the dashboard. Later, launch **MediaBridge** from your application menu or run `bash start-flatpak.sh`. Sign-in startup is requested by default; disable it under **Settings → Startup**. Steam host controls need an additional permission; see **Settings → Steam host access**.

For building, removal and platform limitations, see the [Flatpak guide](packaging/flatpak/README.md).

### Linux — native alternative

Download `install-linux.sh` from [Releases](https://github.com/murasaki-dev/mediabridge/releases) and run:

```sh
bash install-linux.sh
```

This installs the native release and its dependencies. SteamOS/Bazzite require Distrobox and Podman for this option. Remove it with `mediabridge-uninstall`. See [installation details](docs/INSTALLATION.md) for supported distributions and source builds.

### Windows

Download and run `MediaBridge-Setup.exe` from [Releases](https://github.com/murasaki-dev/mediabridge/releases). Setup includes the playback tools and offers launch at sign-in.

## Use it

Open **`http://<mediabridge-pc>:8765/`** from another device on the same network, or `http://127.0.0.1:8765/` on the host. Keep MediaBridge running in the logged-in desktop session.

1. Open **Settings** to choose playback defaults and the server port.
2. Use **Connect your accounts** to sign in through managed LibreWolf. Close it after signing in, reopen the dashboard, and use **Check** to verify cookie access.
3. Paste a link, select browser or local playback, then play it now or add it to the queue.

For Sunshine streaming, select **Sunshine / Moonlight**, start pairing, and enter Moonlight's PIN on the Sunshine host.

The API has **no authentication or HTTPS**. Use it on a trusted LAN; anyone who can reach its port can control the host. The browser-origin allowlist is not user authentication. After changing the port, update the host firewall rule as needed.

## Integrate

```sh
curl -X POST http://127.0.0.1:8765/play \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://example.com/video.mp4","local":true}'

curl http://127.0.0.1:8765/status
curl -X POST http://127.0.0.1:8765/pause
```

The [HTTP API reference](API.md) covers playback, batch queues, metadata/history, autoplay, settings, hardware decoder reporting, and the SMB provider contract. `/play` replaces the queue; `/play_preserve_queue` retains it. `/close` clears playback and the queue while keeping in-memory session history.

## Projects behind MediaBridge

MediaBridge brings together these projects; thanks to their maintainers and contributors:

| Project | Role |
| --- | --- |
| [mpv](https://mpv.io/) and [FFmpeg](https://ffmpeg.org/) | Local playback and media decoding. Windows packaging uses [zhongfly/mpv-winbuild](https://github.com/zhongfly/mpv-winbuild). |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [Deno](https://deno.com/) | Media extraction, metadata, playlists, and JavaScript challenge support. |
| [LibreWolf](https://librewolf.net/), built on [Firefox](https://www.mozilla.org/firefox/), with [uBlock Origin](https://github.com/gorhill/uBlock) | Managed browser playback and persistent sign-in profile. |
| [Moonlight Qt](https://github.com/moonlight-stream/moonlight-qt) and [Sunshine](https://github.com/LizardByte/Sunshine) | Game and desktop streaming; Sunshine runs separately on the streaming host. |
| [Flatpak](https://flatpak.org/) and [Fedora](https://fedoraproject.org/) | Recommended Linux package and its isolated runtime. |
| [Distrobox](https://distrobox.it/) and [Podman](https://podman.io/) | Native runtime on SteamOS/Bazzite and container-based Flatpak builds. |
| [Rust](https://www.rust-lang.org/), [GTK](https://www.gtk.org/), and [tray-icon](https://github.com/tauri-apps/tray-icon) | Application, desktop, and tray foundations. |

See [project credits](THIRD_PARTY.md) for the direct Rust libraries, Linux desktop tools, packaging tools, and optional SMB companion projects. Components retain their own licenses; the credits list is not a replacement for their license notices.
