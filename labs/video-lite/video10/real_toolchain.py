#!/usr/bin/env python3
"""VIDEO-10: prove real yt-dlp + QuickJS + native Rust MP4 mux using localhost DASH.

No YouTube or external media. This script only serves a synthetic MPD generated
from the repository-owned two-second H264/AAC fixture. The download is explicit,
the server is bound to loopback, and the output is disposable CI-only data.
"""
import argparse
import functools
import http.server
import json
import pathlib
import subprocess
import tempfile
import threading

LIMIT = 40 * 1024 * 1024

class Server(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):  # noqa: A002
        return

def run(args, *, cwd, timeout=180):
    cmd = [str(v) for v in args]
    p = subprocess.run(cmd, cwd=str(cwd), text=True,
                       capture_output=True, timeout=timeout, check=False)
    if p.returncode:
        print("VIDEO10_CMD_FAILED:", cmd[0], "code=", p.returncode)
        print((p.stdout + "\n" + p.stderr)[-4000:])
        raise RuntimeError(f"subprocess failed ({p.returncode})")
    return p.stdout, p.stderr

def formats(ytdlp, quickjs, manifest, cwd):
    stdout, _ = run(
        [ytdlp, "--ignore-config", "--no-plugin-dirs",
         "--no-remote-components", "--no-js-runtimes",
         "--js-runtimes", "quickjs:" + str(quickjs),
         "--no-playlist", "--dump-single-json", "--", manifest],
        cwd=cwd, timeout=60)
    data = json.loads(stdout)
    found = data.get("formats") or []
    selected = [
        {"format_id": f.get("format_id"), "ext": f.get("ext"),
         "vcodec": f.get("vcodec"), "acodec": f.get("acodec"),
         "height": f.get("height"), "protocol": f.get("protocol")}
        for f in found]
    print("VIDEO10_FORMATS", json.dumps(selected, separators=(",", ":"))[:5000])
    if not found:
        raise RuntimeError("yt-dlp returned no MPD formats")
    return found

def load_single_track(ytdlp, quickjs, manifest, output_dir, selector, stem):
    output_dir = output_dir.resolve()
    args = [
        ytdlp, "--ignore-config", "--no-plugin-dirs",
        "--no-remote-components", "--no-js-runtimes",
        "--js-runtimes", "quickjs:" + str(quickjs),
        "--no-cache-dir", "--no-cookies", "--no-playlist",
        "--no-exec", "--no-overwrites", "--no-color",
        "--no-write-info-json", "--no-write-thumbnail",
        "--no-write-subs", "--no-write-auto-subs",
        "--no-write-comments", "--no-download-archive",
        "--restrict-filenames", "--newline", "--progress",
        "--no-warnings", "--retries", "1", "--fragment-retries", "1",
        "--socket-timeout", "15", "--fixup", "never",
        "--format", selector, "--output", stem + ".%(ext)s",
        "--paths", "home:" + str(output_dir),
        "--paths", "temp:" + str(output_dir),
        "--max-filesize", str(LIMIT),
        "--progress-template",
        "download:AKULA_PROGRESS:%(progress.downloaded_bytes|0)s:"
        "%(progress.total_bytes|0)s:%(progress.total_bytes_estimate|0)s",
        "--", manifest,
    ]
    out, err = run(args, cwd=output_dir, timeout=90)
    progress = [s for s in out.splitlines() if "AKULA_PROGRESS:" in s]
    print("VIDEO10_" + stem.upper() + "_PROGRESS_EVENTS=", len(progress))
    ext = "mp4" if stem == "video" else "m4a"
    expected = output_dir / (stem + "." + ext)
    if not expected.is_file() or not (0 < expected.stat().st_size < LIMIT):
        print("VIDEO10_UNEXPECTED_OUTPUTS", [p.name for p in output_dir.iterdir()])
        print(err[-1500:])
        raise RuntimeError(f"native yt-dlp didn't write expected {expected.name}")
    return expected

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--asset-root", required=True, type=pathlib.Path)
    p.add_argument("--scratch", required=True, type=pathlib.Path)
    p.add_argument("--yt-dlp", required=True, type=pathlib.Path)
    p.add_argument("--quickjs", required=True, type=pathlib.Path)
    p.add_argument("--mux", required=True, type=pathlib.Path)
    args = p.parse_args()

    root = args.asset_root.resolve()
    temp = args.scratch.resolve()
    temp.mkdir(parents=True, exist_ok=True)
    manifest = root / "manifest.mpd"
    if not manifest.is_file():
        raise RuntimeError("synthetic MPD fixture is missing")
    for binary in (args.yt_dlp, args.quickjs, args.mux):
        if not binary.resolve().is_file():
            raise RuntimeError("explicit executable is unavailable: " + str(binary))
    for name in ("video.mp4", "audio.m4a", "combined.mp4"):
        if (temp / name).exists():
            raise RuntimeError("refusing to reuse existing media artifact: " + name)

    handler = functools.partial(Server, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        url = f"http://127.0.0.1:{server.server_address[1]}/manifest.mpd"
        fmt = formats(args.yt_dlp, args.quickjs, url, temp)
        videos = [f for f in fmt if str(f.get("vcodec", "")).startswith("avc1")
                  and str(f.get("acodec", "none")) == "none"
                  and f.get("ext") == "mp4"
                  and int(f.get("height") or 0) <= 720]
        audio = [f for f in fmt if str(f.get("acodec", "")).startswith("mp4a")
                 and str(f.get("vcodec", "none")) == "none"
                 and f.get("ext") == "m4a"]
        if not videos or not audio:
            raise RuntimeError("MPD lacks exact AVC MP4 and AAC M4A selectors used by Akula")
        v = load_single_track(args.yt_dlp, args.quickjs, url, temp,
                              "bv[vcodec^=avc1][ext=mp4][height<=720]", "video")
        a = load_single_track(args.yt_dlp, args.quickjs, url, temp,
                              "ba[acodec^=mp4a][ext=m4a]", "audio")
        target = temp / "combined.mp4"
        stdout, _ = run([args.mux, "mux-pair", v, a, target], cwd=temp)
        if "DASH_RUST_MUX_PAIR_PASS" not in stdout:
            raise RuntimeError("Rust mux did not report success")
        if not target.is_file() or not (1000 < target.stat().st_size < LIMIT):
            raise RuntimeError("Rust combined MP4 missing or over quota")
        output = {
            "bytes": target.stat().st_size,
            "video_bytes": v.stat().st_size,
            "audio_bytes": a.stat().st_size,
            "no_external_media": True,
            "ffmpeg_used_for_remux": False,
        }
        print("VIDEO10_REAL_TOOLCHAIN_PASS", json.dumps(output, sort_keys=True))
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=10)

if __name__ == "__main__":
    main()
