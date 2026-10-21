# Project credits

MediaBridge depends on the work of these projects and their contributors. This inventory identifies direct application dependencies, tools used by the installers, and optional integrations. It is not a list of everything bundled transitively by upstream binaries or a replacement for their license/copyright notices. Consult the linked projects and shipped notices for their terms.

## Playback and browser runtime

| Project | Use in MediaBridge |
| --- | --- |
| [mpv](https://mpv.io/) | Local playback, extractor integration, IPC controls, and hardware decoding. |
| [FFmpeg](https://ffmpeg.org/) | Media decoding/protocol support in playback builds; capabilities depend on the installed build. |
| [zhongfly/mpv-winbuild](https://github.com/zhongfly/mpv-winbuild) | Windows mpv binaries downloaded by the packaging script. |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | Media extraction, metadata, playlist/Mix expansion, and cookie checks. |
| [yt-dlp EJS](https://github.com/yt-dlp/ejs) | Remote JavaScript challenge components requested through yt-dlp. |
| [Deno](https://github.com/denoland/deno) | JavaScript runtime for yt-dlp challenge handling. |
| [LibreWolf](https://librewolf.net/) | Managed browser and persistent sign-in profile; portable Windows and AppImage Linux distributions. |
| [Firefox](https://www.mozilla.org/firefox/) | LibreWolf's browser foundation; automation uses Firefox's WebDriver BiDi support. |
| [uBlock Origin](https://github.com/gorhill/uBlock) | Browser extension supplied by upstream LibreWolf distributions. |
| [Moonlight Qt](https://github.com/moonlight-stream/moonlight-qt) | Bundled streaming client and pairing flow. |
| [Sunshine](https://github.com/LizardByte/Sunshine) | Compatible streaming host, installed separately on the host PC. |

## Linux desktop and deployment tools

| Project | Use in MediaBridge |
| --- | --- |
| [playerctl](https://github.com/altdesktop/playerctl) / [MPRIS](https://specifications.freedesktop.org/mpris-spec/latest/) | Desktop media controls/status where supported. |
| [xdotool](https://github.com/jordansissel/xdotool) | X11 pointer and window interaction. |
| [ydotool](https://github.com/ReimuNotMoe/ydotool) | Wayland-compatible pointer injection through its daemon. |
| [xdg-utils](https://www.freedesktop.org/wiki/Software/xdg-utils/) | Desktop URL/file opening helpers. |
| [libnotify](https://gitlab.gnome.org/GNOME/libnotify) | Best-effort playback-error notifications through `notify-send`. |
| [GTK](https://www.gtk.org/) / [gtk-rs](https://gtk-rs.org/) | Linux desktop/tray libraries and Rust bindings. |
| [AppIndicator](https://launchpad.net/libappindicator) / [Ayatana AppIndicator](https://github.com/AyatanaIndicators/libayatana-appindicator) | Tray integration; package selection differs by distribution. |
| [Python](https://www.python.org/) and [SecretStorage](https://github.com/mitya57/secretstorage) | Installer helpers and yt-dlp's Linux browser-cookie access. |
| [D-Bus](https://www.freedesktop.org/wiki/Software/dbus/) | Desktop session IPC used by desktop tools. |
| [systemd](https://systemd.io/) | User service, journal, session startup, and host suspend where available. |
| [Distrobox](https://distrobox.it/) and [Podman](https://podman.io/) | Dedicated runtime container on SteamOS/Bazzite. |
| [Fedora](https://fedoraproject.org/) | Container runtime distribution used by that installer path. |
| [AppImage](https://appimage.org/) | Linux browser/streaming-client distribution format; installers extract it. |
| [curl](https://curl.se/) and [GNU Bash](https://www.gnu.org/software/bash/) | Downloading and Linux setup/launcher scripts. |

## Direct Rust dependencies

These are declared in [Cargo.toml](Cargo.toml). [Cargo.lock](Cargo.lock) records the resolved direct and transitive Rust packages.

| Project | Role |
| --- | --- |
| [Serde](https://serde.rs/) / [serde_json](https://github.com/serde-rs/json) | Configuration and JSON API serialization. |
| [RustCrypto sha1](https://github.com/RustCrypto/hashes) | Protocol hashing. |
| [tiny_http](https://github.com/tiny-http/tiny-http) | HTTP server. |
| [tungstenite](https://github.com/snapview/tungstenite-rs) | WebSocket communication for browser automation. |
| [tray-icon](https://github.com/tauri-apps/tray-icon) | Desktop notification-area/tray icon. |
| [winit](https://github.com/rust-windowing/winit) | Desktop event loop/window integration. |
| [windows-sys](https://github.com/microsoft/windows-rs) | Native Windows API bindings. |
| [gtk](https://github.com/gtk-rs/gtk3-rs) | GTK 3 bindings on Linux. |

## Build and packaging

- [Rust, Cargo](https://www.rust-lang.org/) and [rustup](https://rustup.rs/) — compiler, dependency management, and toolchain installation.
- [7-Zip](https://www.7-zip.org/) and [Inno Setup](https://jrsoftware.org/isinfo.php) — Windows extraction and installer creation.
- [Windows Package Manager (winget)](https://github.com/microsoft/winget-cli) — Windows setup/build tool installation.
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) — Windows native compiler/linker toolchain.
- [pkg-config](https://www.freedesktop.org/wiki/Software/pkg-config/) / [pkgconf](https://github.com/pkgconf/pkgconf) — Linux native library discovery during builds.
- Distribution package managers supply the Linux runtime/build libraries; transitive packages vary with distribution and release.

## Optional SMB companion integration

The provider adapter was ported from the separate Baka-Inu integration. Baka-Inu owns its share browser, credentials, permissions, and relay; those components are not shipped by standalone MediaBridge. A native deployment must supply its own compatible provider or use that companion service. See [the provider contract](API.md#smb-credential-providers).

Baka-Inu's relay uses [Samba/libsmbclient](https://www.samba.org/) and [pysmbc](https://github.com/hamano/pysmbc); its browsing integration uses Samba's `smbclient`. These are companion-service dependencies, not unconditional MediaBridge installer dependencies.
