import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import subprocess
import tempfile

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1] / "packaging/flatpak/verify-codecs.py"
spec = importlib.util.spec_from_file_location("verify_codecs", source)
codecs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(codecs)


class CodecChecks(unittest.TestCase):
    def test_decode_failure_is_rejected_even_with_successful_exit(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(codecs.subprocess, "run") as run:
            run.side_effect = [subprocess.CompletedProcess([], 0),
                               subprocess.CompletedProcess([], 0, "Video: no video\n")]
            with self.assertRaisesRegex(RuntimeError, "real hevc decoding"):
                codecs.check_playback(Path(directory))

    def test_video_and_audio_are_decoded_independently_without_hardware(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(codecs.subprocess, "run") as run:
            run.side_effect = [subprocess.CompletedProcess([], 0),
                               subprocess.CompletedProcess([], 0, "VO: [null]"),
                               subprocess.CompletedProcess([], 0),
                               subprocess.CompletedProcess([], 0, "AO: [null]")]
            codecs.check_playback(Path(directory))
            playback = [call.args[0] for call in run.call_args_list if call.args[0][0] == "mpv"]
            self.assertIn("--aid=no", playback[0])
            self.assertIn("--vid=no", playback[1])
            self.assertTrue(all("--hwdec=no" in command for command in playback))

    def test_software_decoders_are_recognized(self):
        codecs.check_decoders("Available video decoders:\n h264 - H.264\n hevc - HEVC\n", ["h264", "hevc"], "video")
        codecs.check_decoders(" ffmpeg:dca - DCA (DTS)\n ac3 - AC-3\n", ["dca", "ac3"], "audio")

    def test_hardware_wrapper_does_not_hide_missing_software_decoder(self):
        with self.assertRaisesRegex(RuntimeError, "hevc"):
            codecs.check_decoders(" hevc_v4l2m2m - HEVC wrapper\n", ["hevc"], "video")

    def test_description_is_not_treated_as_a_decoder_name(self):
        with self.assertRaisesRegex(RuntimeError, "hevc"):
            codecs.check_decoders(" h264 - decode hevc-compatible description\n", ["hevc"], "video")


if __name__ == "__main__":
    unittest.main()
