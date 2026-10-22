# MediaBridge Flatpak

Flatpak is the recommended Linux installation, including SteamOS and Bazzite.
This package includes its own Fedora runtime and playback tools. It is distributed
as local bundles, not through Flathub; package updates are installed manually.

## Install and launch

Download these four files from [Releases](https://github.com/murasaki-dev/mediabridge/releases)
into the same folder, then run `bash install-flatpak.sh` without sudo:

- `MediaBridge.flatpak`
- `MediaBridge-Platform.flatpak`
- `install-flatpak.sh`
- `start-flatpak.sh`

Requires Flatpak on x86_64 Linux. If downloaded as an archive, extract it first.
The helper installs both bundles and launches MediaBridge independently of the
terminal. Install updates the same way; quit and reopen a running copy afterward.

Later, launch **MediaBridge** from your application menu, or run:

```sh
bash start-flatpak.sh                # server + dashboard
bash start-flatpak.sh --background   # server only
```

Sign-in startup is requested by default, subject to desktop approval. Disable it
in **Settings → Startup**; the choice survives upgrades. Sign-in opens neither a
terminal nor the dashboard. GNOME's Background Apps entry opens the dashboard;
its × requests cleanup and quits the app.

Stop a native MediaBridge instance first: both normally use TCP port 8765.
Native settings and browser cookies are not imported.

## Steam access and limitations

**Settings → Steam host access** shows the current permission and setup command:

```sh
flatpak override --user --talk-name=org.freedesktop.Flatpak io.github.murasaki_dev.MediaBridge
```

Quit and reopen afterward. This optional permission allows commands to run as
your user outside the sandbox; MediaBridge cannot grant it to itself. Steam
controls also require host Python 3 and libX11. Playback works without this
permission. Steam and Sunshine are installed separately on their respective hosts.

Steam Deck installation has been tested by the user; Gaming Mode startup remains
unverified. Wayland restrictions still apply. The runtime includes AMD Mesa
Freeworld VA-API, modern/legacy Intel media drivers, the NVIDIA VA-API backend,
and RPM Fusion's full FFmpeg libraries
and verifies common video/audio decoders, including HEVC, AC-3 and DTS. Decoder
inventories are saved under `/app/share/mediabridge/`. External SMB providers are
not bundled. The package does not change firewall rules; allow your configured
TCP port for LAN access.

### Hardware decoding

mpv uses automatic hardware decoding with software fallback. GPU selection is
not forced. On NVIDIA hosts, the installer adds Flathub if needed and installs
the graphics extension matching the host's NVIDIA driver. This needs internet
unless the matching extension is already installed. Host kernel drivers are
still required and are not bundled or changed. After a host NVIDIA driver
upgrade, run `flatpak update --user` or rerun `install-flatpak.sh`, then restart
MediaBridge. Graphics support still needs testing on each target GPU.

Check driver visibility and supported VA-API profiles after installation:

```sh
flatpak run --command=mediabridge-check-gpu io.github.murasaki_dev.MediaBridge
```

The build checks driver files and linked libraries without needing a GPU.
This diagnostic checks the installed sandbox; the dashboard's decoder indicator
during actual playback confirms whether mpv is using hardware decoding.

## Build on Fedora

Copy the complete source folder, including the local build scripts, to x86_64
Fedora. From its root, as your normal user:

```sh
sudo dnf install podman flatpak python3 tar
bash build-flatpak.sh             # build only
# Or: bash build-flatpak.sh --install
```

Only the prerequisite command uses sudo. The disposable Fedora 44 build container
needs internet and several GB of disk space. Both bundles, both helpers and
`build.log` appear under `dist/flatpak/`. Distribute all four release files together.

If downloads work on the host but Podman reports `Network is unreachable`, use:

```sh
MEDIABRIDGE_FLATPAK_NETWORK=host bash build-flatpak.sh
```

This changes build-container networking only. Cargo.lock pins Rust dependencies;
RPMs and tool releases are resolved during the build. Version/hash inventories
are recorded in `/app/share/mediabridge/`. The private runtime makes this prototype
larger than a shared-runtime package.

## Logs and removal

Detached-launch logs default to `~/.local/state/mediabridge-flatpak/launcher.log`.
App settings, profiles and logs live under
`~/.var/app/io.github.murasaki_dev.MediaBridge/`. For foreground diagnostics, run
`flatpak run io.github.murasaki_dev.MediaBridge`.

Discover/GNOME Software can remove the app. The runtime may remain; remove both
explicitly with:

```sh
flatpak uninstall --user io.github.murasaki_dev.MediaBridge
flatpak uninstall --user io.github.murasaki_dev.MediaBridge.Platform
```

Add `--delete-data` to app removal to erase settings and sign-ins. If enabled,
clear the host-access override with
`flatpak override --user --reset io.github.murasaki_dev.MediaBridge`.
