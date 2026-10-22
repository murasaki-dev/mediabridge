#!/usr/bin/python3
"""Read-only target-machine GPU diagnostic, run inside the installed Flatpak."""
import ctypes
import os
from pathlib import Path
import subprocess
import sys


def main():
    print("VA-API search path:", os.environ.get("LIBVA_DRIVERS_PATH", "(driver default)"), flush=True)
    for directory in os.environ.get("LIBVA_DRIVERS_PATH", "").split(":"):
        if directory:
            print("Drivers in", directory, ":", ", ".join(p.name for p in sorted(Path(directory).glob("*_drv_video.so"))), flush=True)
    try:
        ctypes.CDLL("libcuda.so.1")
        print("NVIDIA CUDA driver library is available inside the sandbox.", flush=True)
    except OSError:
        print("NVIDIA CUDA driver library is unavailable (normal on an AMD/Intel-only system).", flush=True)
    devices = sorted(Path("/dev/dri").glob("renderD*"))
    if not devices:
        print("No DRM render nodes are visible to the Flatpak.", flush=True)
    success = False
    for device in devices:
        print(f"\nVA-API capabilities for {device}:", flush=True)
        try:
            result = subprocess.run(["vainfo", "--display", "drm", "--device", str(device)], timeout=20)
            success |= result.returncode == 0
        except (OSError, subprocess.TimeoutExpired) as error:
            print(error, flush=True)
    print("\nThis checks driver access/capabilities, not actual video decoding. Check the dashboard's hardware decoder during playback.", flush=True)
    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
