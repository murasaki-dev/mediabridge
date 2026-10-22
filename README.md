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

### Linux

Run as your normal desktop user on x86_64 glibc Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/murasaki-dev/mediabridge/main/install-linux.sh | bash
```

The installer downloads the latest MediaBridge release, installs runtime dependencies, and adds private LibreWolf and Moonlight AppImages. It supports apt, dnf, pacman, zypper, and xbps. SteamOS and Bazzite use a dedicated Fedora Distrobox with Podman instead of installing packages on the host image.

A published `mediabridge-linux-x86_64.tar.gz` release asset is required. To build and install a checkout instead:

```sh
chmod +x build-linux.sh install-linux.sh
./build-linux.sh
./install-linux.sh --local
```

The builder requires Rust/Cargo and Linux development libraries. Install with `--no-autostart` to disable login startup, or use `--deps-only` for runtime packages only. Remove the app with `mediabridge-uninstall`; add `--purge-data` to remove settings, logs, and browser data too.

For an experimental, isolated Linux package built from a checkout, see the
[local Fedora Flatpak build](packaging/flatpak/README.md). Run
`bash build-flatpak.sh --install` after installing its listed prerequisites.

### Windows

Download `MediaBridge-Setup.exe` from [Releases](https://github.com/murasaki-dev/mediabridge/releases), when available. Setup bundles mpv, yt-dlp, Deno, LibreWolf, and Moonlight; it configures the selected LAN port and optional sign-in startup.

To build from source, run `install-deps.bat`. To build the self-contained installer, run `build-installer.bat`; packaging also downloads the runtime tools and uses 7-Zip and Inno Setup.

See [installation and troubleshooting](docs/INSTALLATION.md) for paths, runtime settings, logs, and platform limitations.

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
| [Distrobox](https://distrobox.it/) and [Podman](https://podman.io/) | Linux runtime isolation on SteamOS/Bazzite. |
| [Rust](https://www.rust-lang.org/), [GTK](https://www.gtk.org/), and [tray-icon](https://github.com/tauri-apps/tray-icon) | Application, desktop, and tray foundations. |

See [project credits](THIRD_PARTY.md) for the direct Rust libraries, Linux desktop tools, packaging tools, and optional SMB companion projects. Components retain their own licenses; the credits list is not a replacement for their license notices.
