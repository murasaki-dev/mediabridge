"""Check the decoders mpv actually exposes, rather than assuming RPM presence."""
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def check_playback(destination):
    """Exercise decoder initialization; decoder listings alone cannot prove it."""
    with tempfile.TemporaryDirectory(prefix="mediabridge-codecs-") as temporary:
        for codec, source, encoder, selection, marker in [
            ("hevc", "color=size=64x64:rate=1:duration=1", "libx265", "--aid=no", "VO:"),
            ("eac3", "anullsrc=r=48000:cl=5.1", "eac3", "--vid=no", "AO:"),
        ]:
            sample = str(Path(temporary) / f"{codec}.mkv")
            encode = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", source,
                      "-t", "0.2", "-threads", "1"]
            encode += (["-c:v", encoder, "-x265-params", "pools=1:frame-threads=1"]
                       if codec == "hevc" else ["-c:a", encoder])
            subprocess.run(encode + ["-y", sample], check=True, timeout=60)
            result = subprocess.run(
                ["mpv", "--no-config", "--hwdec=no", "--vo=null", "--ao=null",
                 "--length=0.2", selection, sample], stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, text=True, timeout=30)
            (destination / f"build-{codec}-playback.txt").write_text(result.stdout)
            if result.returncode or marker not in result.stdout or "Failed to initialize a decoder" in result.stdout:
                raise RuntimeError(f"Flatpak mpv failed real {codec} decoding:\n{result.stdout}")


def check_decoders(output, required, kind):
    missing = [codec for codec in required if not re.search(
        rf"^\s*(?:\S+:)?{re.escape(codec)}(?:\s|$)", output, re.MULTILINE)]
    if missing:
        raise RuntimeError(f"Flatpak mpv is missing {kind} decoders: {', '.join(missing)}")


def main(destination):
    destination = Path(destination)
    for option, codecs, kind in [
        ("--vd=help", ["h264", "hevc", "vp9", "av1"], "video"),
        ("--ad=help", ["aac", "ac3", "eac3", "dca", "flac", "opus"], "audio"),
    ]:
        result = subprocess.run(["mpv", "--no-config", option], stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, check=True)
        (destination / f"build-{kind}-decoders.txt").write_text(result.stdout)
        check_decoders(result.stdout, codecs, kind)
    check_playback(destination)
    print("mpv decoder listings and real HEVC/E-AC-3 software playback passed.", flush=True)


if __name__ == "__main__":
    try:
        main(sys.argv[1])
    except RuntimeError as error:
        raise SystemExit(str(error))
