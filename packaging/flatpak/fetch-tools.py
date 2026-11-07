"""Fetch official tools during the container build; record versions and hashes."""
import hashlib
import http.client
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import zipfile


def fetch(url, destination=None):
    """Retry connection failures and interrupted transfers, never keep partial files."""
    attempts = 4
    for attempt in range(attempts):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "MediaBridge-Flatpak-Builder"})
            with urllib.request.urlopen(req, timeout=60) as response:
                if destination is None:
                    return response.read()
                with destination.open("wb") as output:
                    shutil.copyfileobj(response, output)
            return None
        except (urllib.error.URLError, OSError, EOFError, http.client.HTTPException) as error:
            if destination is not None:
                destination.unlink(missing_ok=True)
            retryable = not isinstance(error, urllib.error.HTTPError) or error.code in (408, 429, 500, 502, 503, 504)
            if not retryable or attempt == attempts - 1:
                raise RuntimeError(
                    f"Could not download {url}: {error}. Check the build container's network, DNS or proxy. "
                    "If the host can reach this URL but rootless Podman cannot, retry with "
                    "MEDIABRIDGE_FLATPAK_NETWORK=host bash build-flatpak.sh --install."
                ) from error
            delay = 2 ** (attempt + 1)
            print(f"Download failed ({attempt + 1}/{attempts}): {url}: {error}; retrying in {delay}s", file=sys.stderr, flush=True)
            time.sleep(delay)


def release(project):
    return json.loads(fetch(f"https://api.github.com/repos/{project}/releases/latest"))


def asset(info, pattern):
    matches = [a for a in info["assets"] if re.fullmatch(pattern, a["name"])]
    if len(matches) != 1:
        raise RuntimeError(f"Expected one {pattern!r} asset in {info['tag_name']}; found {len(matches)}")
    return matches[0]["browser_download_url"]


def main(destination):
    records = []
    destination = Path(destination)
    with tempfile.TemporaryDirectory() as staging:
        staging = Path(staging)

        def download(url, name, version):
            path = staging / name
            print(f"Downloading {name}: {version}", flush=True)
            fetch(url, path)
            with path.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
            records.append({"file": name, "version": version, "url": url, "sha256": digest})
            return path

        def extract_appimage(path, name):
            path.chmod(0o755)
            extract_dir = staging / f"extract-{name}"
            extract_dir.mkdir()
            subprocess.run([str(path), "--appimage-extract"], cwd=extract_dir, check=True, stdout=subprocess.DEVNULL)
            root = extract_dir / "squashfs-root"
            if not (root / "AppRun").is_file():
                raise RuntimeError(f"{name} AppImage has no AppRun")
            shutil.move(str(root), str(destination / "lib" / name))

        moonlight = release("moonlight-stream/moonlight-qt")
        extract_appimage(download(asset(moonlight, r"Moonlight-.+-x86_64\.AppImage"),
                                  "moonlight.AppImage", moonlight["tag_name"]), "moonlight")
        page = fetch("https://librewolf.net/installation/linux/").decode()
        links = re.findall(r'https://dl\.librewolf\.net/librewolf/[^"\s<>]+-linux-x86_64-appimage\.AppImage', page)
        if not links:
            raise RuntimeError("Could not locate the official LibreWolf x86_64 AppImage")
        extract_appimage(download(links[0], "librewolf.AppImage", links[0].rsplit("/", 1)[-1]), "librewolf")

        deno = release("denoland/deno")
        archive = download(asset(deno, r"deno-x86_64-unknown-linux-gnu\.zip"), "deno.zip", deno["tag_name"])
        with zipfile.ZipFile(archive) as bundle:
            with bundle.open("deno") as source, (destination / "bin/deno").open("wb") as output:
                shutil.copyfileobj(source, output)
        (destination / "bin/deno").chmod(0o755)

        ytdlp = release("yt-dlp/yt-dlp")
        binary = download(asset(ytdlp, r"yt-dlp_linux"), "yt-dlp", ytdlp["tag_name"])
        sums = download(asset(ytdlp, r"SHA2-256SUMS"), "yt-dlp-SHA256SUMS", ytdlp["tag_name"])
        expected = next((line.split()[0] for line in sums.read_text().splitlines()
                         if line.split()[-1].lstrip("*") == "yt-dlp_linux"), None)
        if expected != hashlib.sha256(binary.read_bytes()).hexdigest():
            raise RuntimeError("yt-dlp official SHA256SUMS verification failed")
        shutil.copy2(binary, destination / "bin/yt-dlp")
        (destination / "bin/yt-dlp").chmod(0o755)
    (destination / "share/mediabridge/build-downloads.json").write_text(json.dumps(records, indent=2) + "\n")


if __name__ == "__main__":
    try:
        main(sys.argv[1])
    except RuntimeError as error:
        raise SystemExit(str(error))

