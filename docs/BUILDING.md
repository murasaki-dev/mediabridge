# Building release packages

Build from one source checkout and keep `Cargo.toml` and `Cargo.lock` together.
Build scripts and installer definitions are versioned; generated output stays in
`dist/` and compiler caches stay in `target/`.

## Windows

Run `build-installer.bat` from the checkout. It builds the Windows executable,
downloads the bundled tools, and creates `dist/MediaBridge-Setup.exe` using
Inno Setup. It does not install or launch the application.

## Native Linux

On x86_64 Linux with Rust/Cargo installed, run:

```sh
bash build-linux.sh
```

The script checks GTK 3/AppIndicator development dependencies and builds
`dist/mediabridge-linux-x86_64.tar.gz`. Install that exact build with
`bash install-linux.sh --local`. Native binaries require compatible host libraries;
building on a newer distribution does not guarantee support for older glibc.

## Flatpak

On x86_64 Linux with rootless Podman, Flatpak, Python 3, and tar installed:

```sh
bash build-flatpak.sh
```

The builder creates a Fedora-based runtime and app in `dist/flatpak/`. Copy both
`MediaBridge-Platform.flatpak` and `MediaBridge.flatpak`, plus `install-flatpak.sh`
and `start-flatpak.sh`, to the target machine. Run `bash install-flatpak.sh` there.
The installer updates existing installations and preserves application settings.

## Windows using WSL2

Use an x86_64 WSL2 distribution with systemd and keep the Linux build copy in
its Linux filesystem. Install the Linux dependencies there, then run the native
and Flatpak commands above. Windows and Linux must use separate `target/` trees.
For a memory-constrained WSL VM, set `CARGO_BUILD_JOBS=2` before building.
If container networking is unavailable, the Flatpak builder accepts
`MEDIABRIDGE_FLATPAK_NETWORK=host`; this changes only the build container network.

WSL compilation does not verify desktop playback, GPU acceleration, or SMB
connectivity on the target Linux computer. Run those checks on the target host.

