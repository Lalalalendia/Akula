#!/usr/bin/env python3
"""VIDEO-16: run the real Windows Akula GUI against an isolated DASH server.

The application receives native Windows SendInput pointer events. No UI
action is directly invoked by Python, and only a non-shipping Cargo feature
may redirect media transport from its fixed synthetic note URL to loopback.
"""
import argparse
import functools
import hashlib
import http.server
import os
from pathlib import Path
import shutil
import subprocess
import threading

class RestrictedLocalServer(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, directory, allowed, requests_seen, **kwargs):
        self.allowed = allowed
        self.requests_seen = requests_seen
        super().__init__(*args, directory=directory, **kwargs)

    def do_GET(self):
        if self.path not in self.allowed:
            self.send_error(403, "Only bundled synthetic DASH files are served")
            return
        self.requests_seen.append(self.path)
        super().do_GET()

    def log_message(self, fmt, *args):
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    for field in ("repo", "manifest-dir", "exe", "vault", "data", "probe", "output"):
        parser.add_argument("--" + field, type=Path, required=True)
    args = parser.parse_args()
    if (os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("RUNNER_OS") != "Windows"
            or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"):
        raise RuntimeError("VIDEO-16 only runs in hosted Windows CI")
    runner = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    repo = args.repo.resolve(strict=True)
    fixture = args.manifest_dir.resolve(strict=True)
    if not (fixture / "manifest.mpd").is_file():
        raise RuntimeError("Synthetic MPD was not generated")
    allowed = {"/" + p.name for p in fixture.iterdir() if p.is_file()}
    if "/manifest.mpd" not in allowed or len(allowed) < 5:
        raise RuntimeError("The local DASH file set is incomplete")
    for path in (args.vault, args.data, args.probe):
        resolved = path.absolute()
        if not resolved.is_relative_to(runner):
            raise RuntimeError("VIDEO-16 refuses non-disposable paths")
    if args.vault.exists() or args.data.exists() or args.probe.exists():
        raise RuntimeError("The GUI test must start with a completely new profile")
    args.vault.mkdir(parents=True)
    args.data.mkdir(parents=True)
    installed = Path(os.environ["LOCALAPPDATA"]) / "Akula" / "akula-app.exe"
    if args.exe.resolve(strict=True) != installed.resolve(strict=True):
        raise RuntimeError("Unexpected Akula stage path")
    out = args.output.absolute()
    if out.exists() or not out.parent.is_dir():
        raise RuntimeError("Reference output must be new and inside an existing folder")

    requests_seen = []
    handler = functools.partial(
        RestrictedLocalServer, directory=str(fixture),
        allowed=allowed, requests_seen=requests_seen,
    )
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        env = os.environ.copy()
        env["AKULA_VIDEO16_MPD_URL"] = (
            f"http://127.0.0.1:{server.server_address[1]}/manifest.mpd"
        )
        env["AKULA_VIDEO16_PROBE"] = str(args.probe)
        ps = Path(os.environ["SystemRoot"]) / "System32" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
        cmd = [
            str(ps), "-NoProfile", "-NonInteractive", "-Sta",
            "-ExecutionPolicy", "Bypass", "-File",
            str(repo / "scripts/windows-video16-real-gui.ps1"),
            "-ExePath", str(installed), "-VaultPath", str(args.vault),
            "-DataDir", str(args.data), "-ProbePath", str(args.probe),
        ]
        result = subprocess.run(cmd, cwd=repo, env=env, capture_output=True, timeout=540)
        output = (
            result.stdout.decode("utf-8", errors="replace") +
            "\n" + result.stderr.decode("utf-8", errors="replace")
        )
        print(output[-18000:])
        required = [
            "VIDEO16_REAL_MOUSE_DOWNLOAD_PASS",
            "VIDEO16_RESTART_DECODED_GUI_FRAME_PASS",
        ]
        if result.returncode or not all(value in output for value in required):
            raise RuntimeError("VIDEO-16 real GUI acceptance failed")
        source = args.data / "local-videos" / "dQw4w9WgXcQ.mp4"
        if not source.is_file() or source.stat().st_size <= 1000:
            raise RuntimeError("GUI did not persist a playable private MP4")
        if "/manifest.mpd" not in requests_seen or len(requests_seen) < 3:
            raise RuntimeError("Downloader never requested the localhost DASH tracks")
        if not args.probe.is_file():
            raise RuntimeError("No genuine GUI mouse event witness")
        required_proof = {"VIDEO16_HOVER_LOCAL_PLAY", "VIDEO16_CLICK_LOCAL_PLAY", "VIDEO16_DECODED_FRAME"}
        if not required_proof.issubset(set(args.probe.read_text().splitlines())):
            raise RuntimeError("No real post-restart GUI playback proof")
        shutil.copyfile(source, out)
        digest = hashlib.file_digest(out.open("rb"), "sha256").hexdigest()
        print(f"VIDEO16_HOSTED_GUI_E2E_PASS bytes={out.stat().st_size} sha256={digest} "
              f"loopback_requests={len(requests_seen)} third_party_media_requests=0")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=10)


if __name__ == "__main__":
    main()
