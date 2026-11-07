#!/usr/bin/env bash
# Runs ONLY inside the disposable Fedora build container.
set -euo pipefail
[[ -f /run/.containerenv && -d /src/packaging/flatpak ]] || {
  echo 'This helper must run inside build-flatpak.sh, not on the host.' >&2; exit 1;
}
dnf -y --setopt=install_weak_deps=False install \
  ca-certificates curl tar unzip python3 python3-gobject python3-secretstorage \
  mpv playerctl xdotool xdg-utils libnotify gtk3 libappindicator-gtk3 flatpak \
  libX11 libXt dbus-libs alsa-lib pulseaudio-libs mesa-dri-drivers mesa-libEGL \
  mesa-libGL mesa-vulkan-drivers vulkan-loader nss nspr fontconfig dejavu-sans-fonts \
  /usr/bin/flatpak-spawn /usr/bin/prlimit glycin-loaders shared-mime-info glibc-langpack-en \
  cargo rust gcc gcc-c++ make pkgconf-pkg-config gtk3-devel libappindicator-gtk3-devel
# Fedora's default libavcodec omits codecs used by many MKV files. Install the
# full FFmpeg libraries in the disposable runtime, never on the host.
dnf -y install "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm"
dnf -y install "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-$(rpm -E %fedora).noarch.rpm"
dnf -y swap ffmpeg-free ffmpeg --allowerasing
dnf -y --setopt=install_weak_deps=False install \
  mesa-va-drivers-freeworld \
  intel-media-driver libva-intel-driver libva-nvidia-driver libva-utils
ldconfig
# GTK's glycin image loaders spawn a nested Flatpak sandbox. These helpers
# are runtime requirements even though the minimal Fedora image omits them.
command -v flatpak-spawn
command -v prlimit
compgen -G '/usr/libexec/glycin-loaders/*/glycin-svg' >/dev/null
update-mime-database /usr/share/mime
cd /src
mkdir -p /out/app/bin /out/app/lib /out/app/share/mediabridge
# Validate packaging without requiring a GPU in the build container. Actual
# acceleration must be checked on the target GPU with the diagnostic below.
python3 packaging/flatpak/verify-gpu-drivers.py /out/app/share/mediabridge
install -m755 packaging/flatpak/check-gpu.py /out/app/bin/mediabridge-check-gpu
# Fetch before compiling so unavailable upstream/network access fails early.
# Tool downloads are resolved during the build, never on the user's first run.
python3 packaging/flatpak/fetch-tools.py /out/app
cargo build --release --locked
install -m755 target/release/mediabridge /out/app/bin/mediabridge-server
install -m755 packaging/flatpak/launcher.py /out/app/bin/mediabridge-flatpak
install -m644 packaging/flatpak/desktop_input.py /out/app/bin/desktop_input.py
install -m755 packaging/flatpak/librewolf.sh /out/app/bin/mediabridge-librewolf
install -m755 packaging/flatpak/moonlight.sh /out/app/bin/mediabridge-moonlight
install -Dm644 packaging/flatpak/io.github.murasaki_dev.MediaBridge.desktop \
  /out/app/share/applications/io.github.murasaki_dev.MediaBridge.desktop
install -Dm644 packaging/flatpak/io.github.murasaki_dev.MediaBridge.svg \
  /out/app/share/icons/hicolor/scalable/apps/io.github.murasaki_dev.MediaBridge.svg
mkdir -p /out/app/share/dbus-1/services
cat > /out/app/share/dbus-1/services/io.github.murasaki_dev.MediaBridge.service <<'EOF'
[D-BUS Service]
Name=io.github.murasaki_dev.MediaBridge
Exec=/app/bin/mediabridge-flatpak --background
EOF
# Preserve the exact RPM/tool inventory for diagnostics and future version pinning.
rpm -qa --qf '%{NAME} %{VERSION}-%{RELEASE}.%{ARCH}\n' | sort > /out/app/share/mediabridge/build-packages.txt
# Build tools are not used at runtime. Keep their shared libraries for safety in this prototype.
dnf -y remove --no-autoremove cargo rust gcc gcc-c++ make pkgconf-pkg-config gtk3-devel libappindicator-gtk3-devel
ldconfig
# Check the final runtime after dependency removal, including actual decoding.
python3 packaging/flatpak/verify-codecs.py /out/app/share/mediabridge
dnf clean all
rm -rf /src/target

