# Local Fedora Flatpak build (experimental)

Copy the entire source checkout, including Cargo.lock, web/, the Steam source
files and this directory, to an x86_64 Fedora computer. This is a prototype for
local testing, not a published/Flathub package or a validated SteamOS release.

```sh
sudo dnf install podman flatpak python3 tar
bash build-flatpak.sh --install
flatpak run io.github.murasaki_dev.MediaBridge
```

Run from the checkout's root as your normal desktop user. Omit `--install` to
only build. Internet, rootless Podman and several GB of free disk space are
required. Source paths may contain spaces. Nothing is uploaded. Only the first
command installs host packages; the build never uses sudo. If rootless Podman
fails, resolve the error reported by `podman info` before retrying.
Build output and failures are also recorded in `dist/flatpak/build.log`.
Tool downloads retry transient network/transfer failures and run before Rust
compilation. If downloads work on the host but fail inside rootless Podman with
`Network is unreachable`, try
`MEDIABRIDGE_FLATPAK_NETWORK=host bash build-flatpak.sh --install`.
This shares networking only with the disposable build container; it does not
change the installed app's permissions. Check the host connection/proxy first.
`--install` and `install-flatpak.sh` install the two local bundles without further
confirmation prompts, then launch the dashboard/server using the detached
`start-flatpak.sh` helper. Closing the build terminal does not stop that launch.
Omit `--install` if you want to inspect the bundles first.

The script uses a disposable Fedora 44 container, compiles with Cargo.lock,
installs runtime RPMs there, and extracts official LibreWolf/Moonlight AppImages
without FUSE. Deno and yt-dlp are also downloaded during the build. It exports a
private Fedora runtime and the application as two local Flatpak bundles:

```
dist/flatpak/MediaBridge-Platform.flatpak
dist/flatpak/MediaBridge.flatpak
dist/flatpak/install-flatpak.sh
dist/flatpak/start-flatpak.sh
```

Copy all four to another x86_64 Linux computer with Flatpak installed, and run
`bash install-flatpak.sh` there. It installs the runtime first, then the app, for
the current user. These two bundles supply their own runtime: the test computer
does not need Distrobox, Podman, Rust, mpv or a runtime download. The local export
repository is retained under `dist/flatpak/repo` for rebuilding/testing. Updating
means rebuilding and installing both bundles again; there is no update remote.

This intentionally large private runtime keeps Fedora libraries separate from
the host distribution. It is not yet the conventional shared-SDK manifest
proposed earlier. Fedora RPM versions and upstream tool releases are resolved at
build time; repeated builds can differ. Exact package versions, tool URLs/SHA256s
and the container image digest are recorded under `/app/share/mediabridge/` in
the finished app. yt-dlp is checked against its official published checksum list;
the other download hashes are inventory records, not independently verified
signatures. `MEDIABRIDGE_FLATPAK_IMAGE` can select a compatible Fedora image digest
for investigations, but does not pin the RPM repositories or tool releases.

## Running and testing

Launch MediaBridge from GNOME's application grid to run independently of a
terminal. For a detached terminal launch, use `bash dist/flatpak/start-flatpak.sh`
from the checkout, or `bash start-flatpak.sh` alongside the copied bundles. Add
`--background` to start without opening the dashboard. This helper needs no
rebuild; it starts the already installed app in a separate session and writes
output to `${XDG_STATE_HOME:-$HOME/.local/state}/mediabridge-flatpak/launcher.log`.
Bare `flatpak run` below is useful for foreground diagnostics and can stop when
its terminal closes.

Stop a previous native instance first (`systemctl --user stop mediabridge.service`
if installed as a user service). The Flatpak refuses to start if its configured
port is already listening. It does not import native settings or browser cookies.

```sh
flatpak run io.github.murasaki_dev.MediaBridge                 # server + dashboard
flatpak run io.github.murasaki_dev.MediaBridge --background   # server only
flatpak run io.github.murasaki_dev.MediaBridge --quit         # managed cleanup + exit
curl http://127.0.0.1:8765/status
```

The launcher is a single-instance GApplication with a Quit action and a Background
portal request. GNOME can list it under Background Apps and activate the dashboard.
Missing portal support is logged and does not prevent playback. This needs a live
graphical session and an appropriate portal backend; KDE/SteamOS has a different
desktop UI. On first launch after installation, it requests sign-in startup by
default, subject to desktop approval. **Settings → Startup → Start at sign-in**
can disable or re-enable it. The saved opt-out survives upgrades and later
launches. Sign-in launches only the background server, without a dashboard or
terminal window. Disabling startup does not stop the current session. Portal
denials, unsupported desktops and pending approval are shown in the dashboard.
The choice and last confirmed result are stored separately in
`config/mediabridge/autostart.json`, so saving playback preferences does not reset
them. This is desktop-session startup, not a system boot service; Steam Deck
Gaming Mode startup is not verified.

Settings, logs, profiles and cookies live under
`~/.var/app/io.github.murasaki_dev.MediaBridge/`. Tools are read-only under `/app`
and `/usr` inside the sandbox; the host's `/usr` is not used. Logs normally appear
under `data`/`state` according to the Flatpak version's XDG_STATE_HOME, or in the
terminal when launched there. Inspect the actual location with:

```sh
flatpak run --command=sh io.github.murasaki_dev.MediaBridge -c 'echo "$XDG_STATE_HOME"'
```

Test local playback, seeks, track switching, remembered choices, browser sign-in,
browser controls, Moonlight pairing, the close timer and GNOME activation/Quit.
Fedora's packaged mpv/codecs are used; restricted codec availability and AppImage
compatibility need testing. Bundled Mesa is intended for Intel/AMD first. NVIDIA
driver-extension integration has not been added. Wayland pointer/window restrictions
still apply. The package does not modify firewall rules; LAN access may require
allowing TCP 8765 (or your configured port) through the host firewall.

## Optional Steam host control

Steam is a host application. Process inspection/closing uses host Python 3 and
libX11 through `flatpak-spawn --host`. They must exist on the host. Host execution
is **not granted by default**. To opt into this broad permission for testing:

```sh
flatpak override --user --talk-name=org.freedesktop.Flatpak io.github.murasaki_dev.MediaBridge
```

Restart the app afterward. This allows the app to execute commands as you outside
its sandbox; it reduces isolation. Without it, Steam controls report a helper
failure while sandboxed playback remains available. Native Wayland Steam windows
are still unsupported by the existing helper. External SMB providers are not
bundled and must be configured separately; host cookie-store access and Wayland
pointer injection are not set up by this package.

The dashboard also exposes **Settings → Steam host access** with this command and
the current sandbox's effective permission state. It cannot approve its own
permission or modify the host override. Recheck after quitting/reopening the app;
the permission is fixed for each sandbox session.

## Remove

```sh
flatpak uninstall --user io.github.murasaki_dev.MediaBridge
flatpak uninstall --user io.github.murasaki_dev.MediaBridge.Platform
```

Add `--delete-data` to the app removal to erase its settings and browser sign-ins.
If you enabled the optional host override, remove it with
`flatpak override --user --reset io.github.murasaki_dev.MediaBridge`.
The build container is removed automatically; Podman retains the downloaded image
cache. The host prerequisites remain installed. No separate uninstaller is needed.
