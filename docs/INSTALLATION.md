# Installation and operation

For a quick start, see the [README](../README.md). For HTTP requests and preference fields, see [API.md](../API.md).

## Linux installation

Run `install-linux.sh` as the desktop user; it uses `sudo` for system dependencies. Release installation needs a published `mediabridge-linux-x86_64.tar.gz` asset. `./install-linux.sh --local` uses `dist/mediabridge-linux-x86_64.tar.gz`; an explicit archive path is also accepted. Build that archive on Linux with `./build-linux.sh` and an installed Rust/Cargo toolchain.

The builder checks GTK 3/AppIndicator development packages and offers to install missing packages through a supported package manager. `MEDIABRIDGE_INSTALL_BUILD_DEPS=1` skips that confirmation. End users installing a release do not need a compiler. The builder uses the checked-in Cargo lockfile.

The installer installs mpv, yt-dlp, Deno, playerctl, pointer/desktop tools, Python SecretStorage, and desktop/tray libraries. LibreWolf and Moonlight AppImages are downloaded from their upstream sources and extracted to avoid a FUSE requirement. No new Rust dependencies or SMB relay packages are needed for the core queue/metadata/codec changes.

SteamOS/Bazzite need Distrobox and Podman available first. Runtime packages go inside a dedicated `mediabridge-runtime` Fedora container; the installer leaves the host image's package configuration alone. The app, user configuration, browser profile, and service stay in the user's home directory. The uninstaller only removes the container when its MediaBridge ownership marker exists.

Supported release target: x86_64 glibc Linux. ARM and Alpine/musl need separate builds. Native tray support requires a compatible desktop StatusNotifier/AppIndicator host; use the dashboard launcher if no tray icon appears. Run in a graphical desktop session for visible playback. Wayland controls display placement and may ignore requested screen indexes.

## Linux paths and runtime settings

| Item | Default location |
| --- | --- |
| App and extracted tools | `~/.local/share/mediabridge/` |
| Launchers | `~/.local/bin/` |
| Playback/server settings | `~/.config/mediabridge/mediabridge.json` |
| Runtime environment settings | `~/.config/mediabridge/runtime.env` |
| Log | `~/.local/state/mediabridge/mediabridge.log` |
| Managed browser profile | `~/.local/share/mediabridge/librewolf-profile/` |
| User service | `~/.config/systemd/user/mediabridge.service` |

The installer honors `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME`, and records those locations in the generated runtime launcher. Settings from an older configuration beside the executable are migrated by the app when appropriate.

`runtime.env` is trusted shell configuration loaded by `mediabridge-run` for systemd and desktop launches. The installer creates it with mode 0600, preserves its contents on reinstall, and explicitly forwards the supported runtime variables into Distrobox. Do not put passwords or service tokens in it.

```sh
MEDIABRIDGE_LOG_STDERR=1
MEDIABRIDGE_MPV_HWDEC=auto
# Optional: an executable you supply, not a bundled MediaBridge component.
# MEDIABRIDGE_SMB_CREDENTIAL_PROVIDER=/absolute/path/to/smb-provider
# Or configure Baka-Inu's compatible loopback credential service:
# MEDIABRIDGE_SMB_CONFIG_DIR=/absolute/path/to/companion-config
# MEDIABRIDGE_SMB_ROOM_PORT=8081
```

Restart the user service after changing it:

```sh
systemctl --user restart mediabridge.service
systemctl --user status mediabridge.service
journalctl --user -u mediabridge.service -n 80 --no-pager
```

File logging continues; `MEDIABRIDGE_LOG_STDERR=1` also exposes the same diagnostics in the journal. Change it to `0` to disable mirroring. New defaults come from the Rust app: 720p/H.264, with hardware decoding set to `auto`. Reinstalling does not replace existing runtime settings. Directly invoking the raw executable bypasses `runtime.env`; use the generated launcher or supply the variables yourself.

On systems without a systemd user manager, quit the running app and relaunch it through the installed desktop/autostart launcher to reload runtime settings. `--no-autostart` installs without enabling login startup; `--deps-only` installs runtime dependencies without app launchers or runtime configuration.

For Distrobox, provider executables must be executable inside the container, and referenced files must be reachable there. A loopback service must share MediaBridge's network namespace. Installation does not create that service or verify its credentials.

## Windows installation

`build-installer.bat` rebuilds the application, downloads bundled tools, and creates `dist/MediaBridge-Setup.exe`. Network access is required during packaging; the resulting installer is self-contained. Version information comes from `Cargo.toml`. Setup can install 7-Zip and Inno Setup through winget when they are missing.

Setup installs beside the runtime tools under `C:\Program Files\MediaBridge`, creates a private-network firewall rule for the selected port, and optionally starts MediaBridge at sign-in. Settings are stored in `mediabridge.json` beside the executable and preserved on upgrade. Logs are under `%LOCALAPPDATA%\MediaBridge\mediabridge.log`. The tray icon opens the live log console; its menu includes logs and quit controls.

For native SMB providers on Windows, configure the environment of the process launching MediaBridge as described in [the API guide](../API.md#native-executable-provider-contract). The Linux `runtime.env` mechanism does not apply to Windows.

## Accounts, streaming, and troubleshooting

- Use **Connect your accounts** in the dashboard to open sign-in in managed LibreWolf. Sign-in restarts the managed browser without BiDi; close LibreWolf afterward, reopen the dashboard, and use **Check**. Cookie contents stay local. YouTube checks validate extractor access; Twitch checks detect cookie access/presence rather than fully validating the remote session.
- Linux LibreWolf startup diagnostics include its launcher and PID, and preserve launcher stderr for debugging. A browser or notification service must run in the desktop session; desktop notifications are best effort.
- Moonlight is bundled; Sunshine is installed separately on the streaming PC. Pair from the dashboard and enter Moonlight's PIN in Sunshine. `MEDIABRIDGE_MOONLIGHT` or `--moonlight PATH` can select another client.
- X11 pointer control uses xdotool; Wayland uses ydotool. If the installer configures the `mediabridge-input` group, log out and back in to activate membership. Playback does not require rerunning the installer for that change.
- Default dashboard: `http://127.0.0.1:8765/`. From another device, use the PC's LAN address. Check reachability with `curl http://127.0.0.1:8765/status` before diagnosing LAN/firewall issues.
- The installer opens TCP 8765 in recognized active Linux firewalls. Changing the app port does not update that rule; adjust it manually. Neither platform's installer enables unknown or inactive firewall managers.
- Guest SMB needs an SMB-enabled mpv build. Authenticated SMB needs a separately configured provider; relay playback also needs a separate file-serving relay. Installing `libavformat-extra` alone does not guarantee SMB support. See [SMB setup and troubleshooting](../API.md#smb-credential-providers).

## Removal

On Linux, run `mediabridge-uninstall`. It removes launchers, the user service, installed executables, and any marked MediaBridge Distrobox, while keeping configuration (including `runtime.env`), logs, and browser data. `mediabridge-uninstall --purge-data` also removes those personal files. Shared system packages are left installed.

On Windows, use the installed uninstaller. It removes startup integration, firewall rules, installed files, and the bundled browser profile. Optional user-data removal also deletes the app's local settings/log data.
