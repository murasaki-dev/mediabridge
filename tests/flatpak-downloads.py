"""Network retry regression tests; no network requests are made."""
import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1] / "packaging/flatpak/fetch-tools.py"
spec = importlib.util.spec_from_file_location("fetch_tools", source)
tools = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tools)


class Downloads(unittest.TestCase):
    def test_network_failure_retries_then_returns_metadata(self):
        with patch.object(tools.urllib.request, "urlopen", side_effect=[
            urllib.error.URLError(OSError(101, "Network is unreachable")), io.BytesIO(b'{"ok":true}')
        ]) as request, patch.object(tools.time, "sleep") as sleep:
            self.assertEqual(tools.fetch("https://example.test"), b'{"ok":true}')
            self.assertEqual(request.call_count, 2)
            sleep.assert_called_once_with(2)

    def test_permanent_http_error_does_not_retry(self):
        with patch.object(tools.urllib.request, "urlopen", side_effect=urllib.error.HTTPError(
            "https://example.test", 404, "Not found", {}, None
        )) as request, patch.object(tools.time, "sleep") as sleep:
            with self.assertRaisesRegex(RuntimeError, "Could not download"):
                tools.fetch("https://example.test")
            self.assertEqual(request.call_count, 1)
            sleep.assert_not_called()

    def test_partial_download_is_removed_and_restarted(self):
        class Interrupted(io.BytesIO):
            def read(self, count=-1):
                if self.tell():
                    raise OSError("Connection reset")
                return super().read(3)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "tool"
            with patch.object(tools.urllib.request, "urlopen", side_effect=[
                Interrupted(b"partial"), io.BytesIO(b"complete")
            ]), patch.object(tools.time, "sleep"):
                tools.fetch("https://example.test", output)
                self.assertEqual(output.read_bytes(), b"complete")


if __name__ == "__main__":
    unittest.main()
