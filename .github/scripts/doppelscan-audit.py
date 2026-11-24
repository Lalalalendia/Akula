#!/usr/bin/env python3
"""Advisory, commit-pinned Rust clone inventory. No product code or CI gates."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import uuid

VERSION = "0.9.0"
TOOL_URL = ("https://github.com/CMaintz/doppelscan/releases/download/v0.9.0/"
            "doppelscan-v0.9.0-x86_64-unknown-linux-gnu.tar.gz")
TOOL_SHA256 = "e5d5abd63abfb74dddfc52d60d58144f402218aa49b3b6b6e610d1e3171f355f"
IMAGE = "ubuntu:24.04"
SCOPE = re.compile(r"^(aquilum-app/(?:core|native)/src/|modules/[^/]+/src/)")
EXCLUDED_DIRS = {"vendor", "generated", "fixtures", "tests", "test", "probes"}
EXCLUDED_FILE = re.compile(r"^(?:test_.*|.*_tests?\.rs|tests?\.rs|generated\.rs|.*_generated\.rs)$")
COMMON = ["--format", "json", "--limit", "0", "--only", "rust", "--min-tokens", "50"]
MODES = {
    "exact": COMMON + ["--strict"],
    "renamed": COMMON,
    "near": ["near"] + COMMON + ["--similarity", "0.8"],
}


def included(path):
    p = PurePosixPath(path)
    return (not p.is_absolute() and ".." not in p.parts and "\\" not in path
            and not any(ord(c) < 32 or ord(c) == 127 for c in path)
            and bool(SCOPE.match(path)) and path.endswith(".rs")
            and not EXCLUDED_DIRS.intersection(p.parts[:-1])
            and not EXCLUDED_FILE.fullmatch(p.name))


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], timeout=60)


def snapshot(repo, commit, dest):
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("exact Git SHA required")
    if git(repo, "rev-parse", "--verify", commit + "^{commit}").decode().strip() != commit:
        raise ValueError("not the requested commit")
    entries = {}
    for raw in git(repo, "ls-tree", "-r", "-z", "--full-tree", commit).split(b"\0"):
        if not raw:
            continue
        meta, name = raw.split(b"\t", 1)
        mode, kind, oid = meta.decode("ascii").split()
        path = name.decode("utf-8")
        if mode in ("100644", "100755") and kind == "blob" and included(path):
            entries[path] = oid
    if not entries:
        raise ValueError("empty Rust snapshot")
    manifest = {}
    # Read the immutable blobs, not a possibly dirty checkout or a symlink.
    with subprocess.Popen(["git", "-C", str(repo), "cat-file", "--batch"],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as proc:
        try:
            for path, oid in sorted(entries.items()):
                proc.stdin.write((oid + "\n").encode("ascii"))
                proc.stdin.flush()
                header = proc.stdout.readline().decode("ascii").split()
                if len(header) != 3 or header[:2] != [oid, "blob"]:
                    raise ValueError("unexpected Git blob header")
                size = int(header[2])
                if not 0 <= size <= 16 * 1024 * 1024:
                    raise ValueError("Rust source exceeds snapshot size limit")
                data = proc.stdout.read(size)
                if len(data) != size or proc.stdout.read(1) != b"\n":
                    raise ValueError("incomplete Git blob")
                file = dest / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(data)
                manifest[path] = hashlib.sha256(data).hexdigest()
        finally:
            proc.stdin.close()
        if proc.wait(timeout=10):
            raise ValueError("git cat-file failed")
    return manifest


def install_tool(temp):
    archive = temp / "tool.tar.gz"
    subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location",
                    "--retry", "3", "--max-time", "120", TOOL_URL, "-o", str(archive)],
                   check=True, timeout=150)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != TOOL_SHA256:
        raise ValueError("Doppelscan archive SHA-256 mismatch")
    tool_dir = temp / "bin"
    tool_dir.mkdir()
    # Extract only the one expected regular file; never unpack arbitrary paths.
    with tarfile.open(archive, "r:gz") as tar:
        matches = [m for m in tar.getmembers()
                   if m.isfile() and PurePosixPath(m.name).name == "doppelscan"]
        if len(matches) != 1 or not 0 < matches[0].size <= 64 * 1024 * 1024:
            raise ValueError("unexpected analyzer archive")
        with tar.extractfile(matches[0]) as src, (tool_dir / "doppelscan").open("wb") as out:
            shutil.copyfileobj(src, out)
    (tool_dir / "doppelscan").chmod(0o755)
    return tool_dir


def run_mode(mode, args, source, tools, image_id, report):
    name = "akula-doppelscan-" + uuid.uuid4().hex
    cmd = ["docker", "run", "--name", name, "--rm", "--network", "none",
           "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
           "--pids-limit", "256", "--cpus", "2", "--memory", "4g",
           "--user", f"{os.getuid()}:{os.getgid()}",
           "--tmpfs", "/tmp:rw,nosuid,nodev,size=128m", "-e", "HOME=/tmp",
           "-v", str(source) + ":/src:ro", "-v", str(tools) + ":/tools:ro",
           "-w", "/src", image_id, "/tools/doppelscan", *args]
    started = time.monotonic()
    try:
        with (report / (mode + ".stdout")).open("wb") as out, \
             (report / (mode + ".stderr")).open("wb") as err:
            result = subprocess.run(cmd, stdout=out, stderr=err, timeout=300)
        status = result.returncode
    except subprocess.TimeoutExpired:
        status = 124
    finally:
        subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=20, check=False)
    elapsed = round(time.monotonic() - started, 3)
    with (report / "status.jsonl").open("a", encoding="utf-8") as out:
        out.write(json.dumps({"mode": mode, "exit": status, "wall_seconds": elapsed}) + "\n")
    if status:
        raise RuntimeError(f"Doppelscan {mode} failed: exit {status}")


def normalized_location(value, manifest):
    if value.startswith("/src/"):
        value = value[5:]
    if value.startswith("./"):
        value = value[2:]
    if value not in manifest:
        raise ValueError("scanner reported a file outside the source manifest")
    return value


def validate_report(data, manifest):
    stats, groups = data["summary"], data["groups"]
    if not isinstance(groups, list) or stats["groups"] != len(groups):
        raise ValueError("invalid clone-group count")
    if type(stats["files"]) is not int or not 0 < stats["files"] <= len(manifest):
        raise ValueError("invalid scanned-file count")
    top = []
    for index, group in enumerate(groups):
        locations = []
        if len(group["occurrences"]) < 2:
            raise ValueError("invalid clone group")
        for occurrence in group["occurrences"]:
            path = normalized_location(occurrence["file"], manifest)
            start, end = occurrence["start_line"], occurrence["end_line"]
            if type(start) is not int or type(end) is not int or not 1 <= start <= end:
                raise ValueError("invalid source range")
            locations.append({"file": path, "start": start, "end": end})
        if index < 5:
            top.append({"id": group["id"], "tokens": group["tokens"],
                        "score": group["score"], "copies": len(locations),
                        "locations": locations[:4]})
    return {"stats": stats, "top": top, "unscanned_count": len(manifest) - stats["files"]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ref", required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    report = args.report.resolve()
    report.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="akula-clones-", dir=os.environ.get("RUNNER_TEMP")) as tmp:
        temp = Path(tmp)
        source = temp / "src"
        source.mkdir()
        manifest = snapshot(Path.cwd(), args.ref, source)
        inventory = "".join(f"{digest}  {path}\n" for path, digest in manifest.items())
        (report / "input.sha256").write_text(inventory, encoding="utf-8")
        print(f"AKULA_DOPPELSCAN_INPUT commit={args.ref} files={len(manifest)}")
        tools = install_tool(temp)
        subprocess.run(["docker", "pull", IMAGE], check=True, timeout=180)
        image_id = subprocess.check_output(["docker", "image", "inspect", "--format",
                                            "{{.Id}}", IMAGE], timeout=30).decode().strip()
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
            raise ValueError("invalid container image ID")
        run_mode("version", ["--version"], source, tools, image_id, report)
        if (report / "version.stdout").read_text().strip() != "doppelscan " + VERSION:
            raise ValueError("unexpected Doppelscan version")
        modes = {}
        for mode, flags in MODES.items():
            run_mode(mode, flags, source, tools, image_id, report)
            data = json.loads((report / (mode + ".stdout")).read_text())
            modes[mode] = validate_report(data, manifest)
            if modes[mode]["unscanned_count"]:
                print(f"::warning::{mode}: scanner skipped some manifest files")
            # Only validated paths/ranges and counts in CI logs; not source fragments.
            print("AKULA_DOPPELSCAN_MODE " + json.dumps({"mode": mode, **modes[mode]},
                                                      ensure_ascii=True))
        result = {"commit": args.ref, "version": VERSION, "input_files": len(manifest),
                  "manifest_sha256": hashlib.sha256(inventory.encode()).hexdigest(),
                  "tool_sha256": TOOL_SHA256, "image_id": image_id,
                  "settings": MODES, "modes": modes}
        (report / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
        print("AKULA_DOPPELSCAN_PASS " + json.dumps({
            "commit": args.ref, "files": len(manifest),
            "groups": {mode: value["stats"]["groups"] for mode, value in modes.items()}
        }))
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as out:
                out.write("\n### Doppelscan advisory (not a merge gate)\n\n")
                out.write(f"Commit: {args.ref}; selected Rust files: {len(manifest)}.\n\n")
                out.write("| Mode | Scanned | Groups | Scanner ms |\n|---|---:|---:|---:|\n")
                for mode, value in modes.items():
                    stats = value["stats"]
                    out.write(f"| {mode} | {stats['files']} | {stats['groups']} | {stats['elapsed_ms']} |\n")
                out.write("\nCounts overlap and are candidates, not automatic refactoring decisions.\n")


if __name__ == "__main__":
    main()
