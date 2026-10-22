import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1] / "packaging/flatpak/verify-gpu-drivers.py"
spec = importlib.util.spec_from_file_location("verify_gpu", source)
gpu = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gpu)


class DriverChecks(unittest.TestCase):
    def test_driver_search_prefers_full_codec_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            roots = [Path(directory) / name for name in ("dri-freeworld", "dri-nonfree", "dri")]
            for root in roots:
                root.mkdir()
            (roots[0] / gpu.DRIVERS["amd"]).touch()
            (roots[2] / gpu.DRIVERS["amd"]).touch()
            (roots[1] / gpu.DRIVERS["intel"]).touch()
            for name in ("intel_legacy", "nvidia"):
                (roots[2] / gpu.DRIVERS[name]).touch()
            found = gpu.find_drivers(roots)
            self.assertEqual(Path(found["amd"]).parent, roots[0])
            self.assertEqual(Path(found["intel"]).parent, roots[1])

    def test_missing_driver_fails_packaging(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "Missing amd"):
                gpu.find_drivers([Path(directory)])


if __name__ == "__main__":
    unittest.main()
