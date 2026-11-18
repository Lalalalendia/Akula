#!/usr/bin/env python3
"""Locate precisely one Cargo-built executable for a protected Linux test.

The test executable is parsed from Cargo JSON compiler-artifact messages, not
guessed from a glob. Refuse zero/multiple candidates, wrong targets, paths
outside the isolated Cargo artifact tree, symlinks and non-executable files.
This helper performs no privileged operation; workflow jobs run the result
under their existing root restrictions.
"""
import json
import os
import stat
import subprocess
import sys
from pathlib import Path


def fail(message: str) -> None:
    print(f"AKULA_ROOT_TEST_DISCOVERY_FAILED: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    if len(sys.argv) != 3:
        fail("expected exact test target and existing feature name")
    name, feature = sys.argv[1:]
    if not name.startswith("server_bootstrap_") or not all(c.isalnum() or c == "_" for c in name):
        fail("invalid target name")
    if not feature.startswith("server-bootstrap-") or not all(c.isalnum() or c in "-_" for c in feature):
        fail("invalid feature name")

    root = Path(__file__).resolve().parents[2]
    artifact_root = (root / ".artifacts" / "cargo-native").resolve(strict=True)
    workdir = root / "aquilum-app"
    args = [
        "cargo", "test", "--locked", "-p", "aquilum-core",
        "--features", feature, "--test", name,
        "--no-run", "--message-format=json",
    ]
    run = subprocess.run(args, cwd=workdir, capture_output=True, text=True, check=False)
    if run.stderr:
        sys.stderr.write(run.stderr)
    if run.returncode:
        fail("cargo test --no-run failed")

    candidates: set[Path] = set()
    for line in run.stdout.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get("reason") != "compiler-artifact":
            continue
        target = item.get("target", {})
        if target.get("name") != name or "test" not in target.get("kind", []):
            continue
        executable = item.get("executable")
        if executable:
            candidates.add(Path(executable))
    if len(candidates) != 1:
        fail(f"expected one exact compiled test executable; found {len(candidates)}")

    candidate = candidates.pop()
    if not candidate.is_absolute():
        candidate = workdir / candidate
    if candidate.is_symlink():
        fail("test executable path must not be a symlink")
    try:
        real = candidate.resolve(strict=True)
        info = real.stat()
        real.relative_to(artifact_root)
    except (OSError, ValueError) as exc:
        fail(f"test executable not in trusted Cargo artifact tree: {exc}")
    if not stat.S_ISREG(info.st_mode) or not os.access(real, os.X_OK):
        fail("test executable not a regular executable file")
    print(str(real))


if __name__ == "__main__":
    main()
