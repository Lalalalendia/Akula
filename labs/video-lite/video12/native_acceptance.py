#!/usr/bin/env python3
"""Drive Akula's *native* yt-dlp/QuickJS/Rust workflow using synthetic local DASH.

Unlike VIDEO-10's independent Python orchestrator, this invokes the real
aquilum-native Rust unit test, which calls its own two-track downloader and
publication code. The note remains a validated YouTube URL for deduplication;
the transport is a private loopback MPD only inside this test harness.
"""
import argparse
import functools
import hashlib
import http.server
import os
from pathlib import Path
import subprocess
import threading

YTDLP_SHA = "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d"
QUICKJS_SHA = "ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7"

class LocalOnly(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):  # noqa: A002
        return

def hash_and_check(path: Path, sha: str) -> None:
    observed = hashlib.file_digest(path.open("rb"), "sha256").hexdigest()
    if observed != sha:
        raise RuntimeError(f"pin failed for {path.name}: {observed}")

def main() -> None:
    cli = argparse.ArgumentParser()
    cli.add_argument("--repo", type=Path, required=True)
    cli.add_argument("--manifest-dir", type=Path, required=True)
    cli.add_argument("--yt-dlp", type=Path, required=True)
    cli.add_argument("--quickjs", type=Path, required=True)
    cli.add_argument("--mux", type=Path, required=True)
    cli.add_argument("--output", type=Path, required=True)
    args = cli.parse_args()

    repo = args.repo.resolve(strict=True)
    root = args.manifest_dir.resolve(strict=True)
    assert (root / "manifest.mpd").is_file()
    for path in (args.yt_dlp, args.quickjs, args.mux):
        assert path.is_file(), f"missing executable: {path}"
    hash_and_check(args.yt_dlp, YTDLP_SHA)
    hash_and_check(args.quickjs, QUICKJS_SHA)
    out = args.output.resolve()
    if out.exists() or not out.parent.is_dir():
        raise RuntimeError("VIDEO-12 refuses to overwrite an output or create a new directory")
    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(LocalOnly, directory=str(root))
    )
    server.daemon_threads = True
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        env = os.environ.copy()
        env.update({
            "AKULA_VIDEO12_MPD_URL": f"http://127.0.0.1:{server.server_address[1]}/manifest.mpd",
            "AKULA_VIDEO12_YTDLP_BIN": str(args.yt_dlp.resolve()),
            "AKULA_VIDEO12_QUICKJS_BIN": str(args.quickjs.resolve()),
            "AKULA_VIDEO12_RUST_MUX_BIN": str(args.mux.resolve()),
            "AKULA_VIDEO12_CAPTURE": str(out),
        })
        command = [
            "cargo", "test", "--locked",
            "--manifest-path", str(repo / "aquilum-app" / "Cargo.toml"),
            "-p", "aquilum-native", "--bin", "aquilum-native",
            "--features", "installed",
            "video::light::tests::real_windows_native_pipeline_remuxes_local_dash_without_syncing_media",
            "--", "--exact", "--nocapture",
        ]
        result = subprocess.run(command, cwd=repo, env=env,
                                capture_output=True, text=True, timeout=3600)
        combined = result.stdout + "\n" + result.stderr
        print(combined[-12_000:])
        if result.returncode != 0 or "VIDEO12_NATIVE_REAL_TOOLCHAIN_PASS" not in combined:
            raise RuntimeError("native Akula integration did not pass; see Rust test output")
        if not out.is_file() or out.stat().st_size < 1_000:
            raise RuntimeError("real native test did not capture valid synthetic local MP4")
        print(f"VIDEO12_NATIVE_CAPTURE_PASS bytes={out.stat().st_size} external_media_requests=0")
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=10)

if __name__ == "__main__":
    main()
