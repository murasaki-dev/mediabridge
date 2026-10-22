"""Validate bundled VA-API files/libraries without requiring build-host GPU access."""
import json
from pathlib import Path
import subprocess
import sys

DRIVERS = {"amd": "radeonsi_drv_video.so", "intel": "iHD_drv_video.so",
           "intel_legacy": "i965_drv_video.so", "nvidia": "nvidia_drv_video.so"}
SEARCH = [Path("/usr/lib64/dri-freeworld"), Path("/usr/lib64/dri-nonfree"), Path("/usr/lib64/dri")]


def find_drivers(directories):
    found = {}
    for vendor, name in DRIVERS.items():
        driver = next((directory / name for directory in directories if (directory / name).is_file()), None)
        if driver is None:
            raise RuntimeError(f"Missing {vendor} VA-API driver: {name}")
        found[vendor] = str(driver)
    if "dri-freeworld" not in Path(found["amd"]).parts:
        raise RuntimeError("AMD VA-API driver must come from the full-codec dri-freeworld directory")
    return found


def main(destination):
    drivers = find_drivers(SEARCH)
    for driver in drivers.values():
        result = subprocess.run(["ldd", driver], capture_output=True, text=True, check=True)
        if "not found" in result.stdout:
            raise RuntimeError(f"Unresolved dependencies for {driver}:\n{result.stdout}")
    (Path(destination) / "build-gpu-drivers.json").write_text(json.dumps(drivers, indent=2) + "\n")
    print("AMD, Intel and NVIDIA VA-API driver packaging checks passed; target-GPU testing is still required.", flush=True)


if __name__ == "__main__":
    try:
        main(sys.argv[1])
    except RuntimeError as error:
        raise SystemExit(str(error))
