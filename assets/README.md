# MediaBridge icon

`mediabridge-logo-source.png` is the approved bridge/M and filmstrip artwork, generated with the built-in image tool and selected by the project owner. Prompt: preserve option B's two black bridge towers and M-shaped suspension curve on white; replace the span with movie-frame windows and sprocket perforations along both edges.

`mediabridge.png` normalizes the artwork to a 512px application icon. `mediabridge.ico` contains 16, 24, 32, 48, 64, 128 and 256px images. `tray-32.rgba` is the same artwork as 32px RGBA bytes, embedded by both tray implementations. `windows.rc` embeds the ICO when the Windows build script links the executable. The Flatpak SVG wraps the PNG for desktop identity and GNOME Background Apps. The dashboard embeds the PNG/ICO in the server.

Local regeneration: `powershell -ExecutionPolicy Bypass -File build-icons.ps1`. The build script remains excluded from Git by project preference; generated assets are essential source files and remain in Git. Windows packaging must use `install-deps.bat` / `build-installer.ps1` for executable resources; a direct `cargo build` does not embed Windows resources.
