#!/usr/bin/env python3
"""RELEASE-APPIMAGE-REAL-E2E-01: two genuine AppImage binaries, CI-only.

This program is not a publisher, does not activate Akula's updater and writes
only to disposable /tmp/akula-release-e2e-* directories inside Docker.
v1 is the normal 0.3.13 AppImage; v2 is independently recompiled and packaged
with a *CI-only* 0.3.14 version. No signing root or release is shipped.
"""
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

PROTO = "AKULA-LINUX-FIRST-GUI-V1"
NONCE = "a87c6520d3e9416fa5b32897c6d0e124"
VAULT = Path("/tmp/vault/First.md")
AQUILUM = [
    Path("/usr/bin/aquilum-app"),
    Path("/usr/share/applications/Aquilum.desktop"),
    Path("/tmp/user/.local/share/com.dmitriy.aquilum-app/keep"),
]


def require(condition, why):
    if not condition:
        raise RuntimeError("AKULA_REAL_APPIMAGE_E2E_HOLD: " + why)


def shell(*args, check=True, **kwargs):
    return subprocess.run(args, check=check, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True, **kwargs)


def digest(path):
    state = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            state.update(block)
    return state.hexdigest()


def visible():
    return "Akula" in shell("wmctrl", "-l", check=False).stdout


def wait_visible(proc, timeout=55):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if proc.poll() is not None:
            raise RuntimeError("AppImage GUI exited prematurely: " + str(proc.returncode))
        if visible():
            return
        time.sleep(0.35)
    raise RuntimeError("real AppImage GUI never opened an Akula window")


def close_gui(proc):
    require(visible(), "cannot close GUI that was never displayed")
    shell("wmctrl", "-c", "Akula")
    try:
        result = proc.wait(timeout=35)
        require(result == 0, "AppImage returned abnormal GUI exit " + str(result))
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.wait(timeout=10)
        raise RuntimeError("AppImage GUI refused clean exit")


def process_descends(child, root):
    for _ in range(12):
        if child == root:
            return True
        if child <= 1:
            return False
        lines = Path(f"/proc/{child}/status").read_text().splitlines()
        entries = [line.split(":", 1)[1].strip()
                   for line in lines if line.startswith("PPid:")]
        if len(entries) != 1:
            return False
        child = int(entries[0])
    return False


def gui_env():
    env = dict(os.environ)
    for item in (
        "AKULA_LINUX_UPDATE_ACK_FD",
        "AKULA_LINUX_UPDATE_NONCE",
        "AKULA_LINUX_UPDATE_PARENT_PID",
        "AKULA_LINUX_UPDATE_IMAGE_SHA256",
    ):
        env.pop(item, None)
    return env


def launch_gui(image, root, name, overrides=None, pass_fds=()):
    log = (root / f"{name}.log").open("wb")
    env = gui_env()
    if overrides:
        env.update(overrides)
    proc = subprocess.Popen(
        [str(image), "--vault", "/tmp/vault", "--open", str(VAULT)],
        env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        start_new_session=True, pass_fds=pass_fds)
    return proc, log


def check_unsigned_gui_crash(image, root):
    # Deliberately provide a complete but *wrong-supervisor* challenge.
    # Even though the real v2 GUI presents a window, it must never ACK.
    parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    parent.settimeout(5)
    overrides = {
        "AKULA_LINUX_UPDATE_ACK_FD": str(child.fileno()),
        "AKULA_LINUX_UPDATE_NONCE": secrets.token_hex(16),
        "AKULA_LINUX_UPDATE_IMAGE_SHA256": digest(image),
        "AKULA_LINUX_UPDATE_PARENT_PID": str(os.getpid() + 1),
    }
    proc, logfile = launch_gui(image, root, "v2-no-ack", overrides, (child.fileno(),))
    child.close()
    try:
        wait_visible(proc)
        # The GUI sends its refusal asynchronously after first presentation.
        # Require a real rejection witness BEFORE sending SIGKILL.
        proof = root / "v2-no-ack.log"
        for _ in range(60):
            if "wrong OS supervisor" in proof.read_text(errors="replace"):
                break
            require(proc.poll() is None, "v2 exited before supervisor rejection")
            time.sleep(0.25)
        require("wrong OS supervisor" in proof.read_text(errors="replace"),
                "visible v2 did not exercise the wrong-supervisor refusal")
        os.killpg(proc.pid, signal.SIGKILL)
        require(proc.wait(timeout=12) != 0, "SIGKILL did not terminate v2")
        with parent:
            require(parent.recv(256) == b"", "forged supervisor produced a GUI ACK")
        for _ in range(40):
            if not visible():
                break
            time.sleep(0.25)
        require(not visible(), "killed AppImage left an unexpected GUI window")
        print("AKULA_REAL_APPIMAGE_NO_ACK_CRASH_PASS real_v2_gui=1 real_sigkill=1 forged_supervisor_refused=1", flush=True)
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
        logfile.close()
        parent.close()


def check_real_gui_ack(image, root):
    parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    parent.settimeout(45)
    challenge = secrets.token_hex(16)
    signed_digest = digest(image)
    overrides = {
        "AKULA_LINUX_UPDATE_ACK_FD": str(child.fileno()),
        "AKULA_LINUX_UPDATE_NONCE": challenge,
        "AKULA_LINUX_UPDATE_IMAGE_SHA256": signed_digest,
        "AKULA_LINUX_UPDATE_PARENT_PID": str(os.getpid()),
    }
    proc, logfile = launch_gui(image, root, "v2-valid-ack", overrides, (child.fileno(),))
    child.close()
    try:
        wait_visible(proc)
        with parent.makefile("rb") as stream:
            line = stream.readline(280)
        parts = line.decode("ascii", "strict").rstrip("\n").split("\t")
        require(len(parts) == 4 and parts[0] == PROTO, "unexpected ACK wire format")
        require(parts[1] == challenge and parts[3] == signed_digest,
                "ACK nonce/hash mismatch")
        gui_pid = int(parts[2])
        require(process_descends(gui_pid, proc.pid), "ACK from other process tree")
        require(os.getpgid(gui_pid) == proc.pid,
                "ACK not from the isolated AppImage runtime session")
        close_gui(proc)
        print("AKULA_REAL_APPIMAGE_V2_ACK_PASS visible_frame=1 pid_tree=1 nonce=1 sha256=1", flush=True)
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
        parent.close()
        logfile.close()


def signed_lab_manifest(root, image, sha):
    # This is an isolated integration fixture, not a release signing key.
    source = os.environ.get("AKULA_E2E_SOURCE_COMMIT", "")
    locked = os.environ.get("AKULA_E2E_LOCK_SHA256", "")
    require(len(source) == 40 and all(c in "0123456789abcdef" for c in source),
            "exact checked-out source SHA was not supplied")
    require(len(locked) == 64 and all(c in "0123456789abcdef" for c in locked),
            "exact patched v2 Cargo.lock SHA-256 was not supplied")
    manifest = {
        "schema": 1, "product": "Akula", "version": "0.3.14",
        "source_commit": source, "cargo_lock_sha256": locked,
        "artifacts": {
            "windows-x86_64": {"filename": "Akula_0.3.14_x64-setup.exe",
                               "sha256": "c" * 64, "size": 12},
            "linux-x86_64-appimage": {
                "filename": image.name, "sha256": sha,
                "size": image.stat().st_size
            },
            "linux-x86_64-deb": {"filename": "Akula_0.3.14_amd64.deb",
                                 "sha256": "c" * 64, "size": 12},
            "linux-x86_64-rpm": {"filename": "Akula-0.3.14-1.x86_64.rpm",
                                 "sha256": "c" * 64, "size": 12}
        }
    }
    path = root / "manifest.json"
    path.write_text(json.dumps(manifest, separators=(",", ":")))
    secret, public, signature = root / "test.key", root / "test.pub", root / "manifest.minisig"
    wrong_secret, wrong_public = root / "wrong.key", root / "wrong.pub"
    shell("minisign", "-G", "-W", "-s", str(secret), "-p", str(public))
    shell("minisign", "-G", "-W", "-s", str(wrong_secret), "-p", str(wrong_public))
    shell("minisign", "-S", "-q", "-s", str(secret), "-m", str(path),
          "-x", str(signature))
    shell("minisign", "-Vm", str(path), "-x", str(signature),
          "-p", str(public), "-Q")
    require(shell("minisign", "-Vm", str(path), "-x", str(signature),
                  "-p", str(wrong_public), "-Q", check=False).returncode != 0,
            "wrong test publisher key unexpectedly verified")
    changed = root / "modified-manifest.json"
    changed.write_bytes(path.read_bytes() + b" ")
    require(shell("minisign", "-Vm", str(changed), "-x", str(signature),
                  "-p", str(public), "-Q", check=False).returncode != 0,
            "modified manifest unexpectedly verified")
    require(digest(image) == sha, "candidate changed after ephemeral signature")
    print("AKULA_REAL_APPIMAGE_CLI_SIGNATURE_PASS genuine_minisign=1 wrong_key_refused=1 altered_metadata_refused=1", flush=True)


def main():
    require(len(sys.argv) == 4, "expected old and new real AppImages and harness")
    require(os.environ.get("GITHUB_ACTIONS") == "true"
            and os.environ.get("AKULA_DISPOSABLE_UPDATE_LAB") == "1",
            "CI-only real package experiment is disabled")
    v1, v2, harness = map(Path, sys.argv[1:])
    require(v1.name == "akula-app_0.3.13_x86_64.AppImage", "wrong compiled v1 identity")
    require(v2.name == "akula-app_0.3.14_x86_64.AppImage", "wrong compiled v2 identity")
    require(all(x.is_file() for x in (v1, v2, harness)), "missing real binaries")
    old_hash, new_hash = digest(v1), digest(v2)
    require(old_hash != new_hash and len(old_hash) == 64 and len(new_hash) == 64,
            "the two AppImages were not separately compiled")
    watch = {f: digest(f) for f in [VAULT] + AQUILUM}
    with tempfile.TemporaryDirectory(prefix="akula-release-e2e-", dir="/tmp") as root_name:
        root = Path(root_name)
        signed_lab_manifest(root, v2, new_hash)
        # Negative flow: install v2 bytes, show actual GUI, kill it before
        # valid ACK, cold-rollback old executable, and launch the restored v1.
        failure = root / "failure"
        failure.mkdir()
        # Helper is intentionally restricted to an immediate /tmp root;
        # keep all transaction states in independent top-level directories.
        for case in ("negative", "positive"):
            trial = Path(tempfile.mkdtemp(prefix="akula-release-e2e-", dir="/tmp"))
            try:
                installed = trial / v1.name
                shutil.copy2(v1, installed)
                require(digest(installed) == old_hash, "initial v1 copy differs")
                before = watch.copy()
                shell(str(harness), "publish", str(installed), str(v2),
                      env={**os.environ, "AKULA_DISPOSABLE_UPDATE_LAB": "1"})
                require(digest(installed) == new_hash, "v2 publication bytes differ")
                backup = trial / (".akula-appimage-update." + NONCE) / "previous.AppImage"
                require(digest(backup) == old_hash, "previous v1 was not retained")
                if case == "negative":
                    check_unsigned_gui_crash(installed, trial)
                    shell(str(harness), "rollback", str(installed),
                          env={**os.environ, "AKULA_DISPOSABLE_UPDATE_LAB": "1"})
                    require(digest(installed) == old_hash, "old v1 not restored")
                    failed = backup.parent / "failed.AppImage"
                    require(digest(failed) == new_hash, "failed real v2 bytes lost")
                    proc, logfile = launch_gui(installed, trial, "restored-v1")
                    try:
                        wait_visible(proc)
                        close_gui(proc)
                    finally:
                        if proc.poll() is None:
                            os.killpg(proc.pid, signal.SIGKILL)
                            proc.wait()
                        logfile.close()
                    print("AKULA_REAL_APPIMAGE_V1_RECOVERED_PASS actual_gui=1 v1_sha256=1 v2_failed_preserved=1", flush=True)
                else:
                    check_real_gui_ack(installed, trial)
                    require(digest(backup) == old_hash, "successful ACK removed previous v1")
                    require(digest(installed) == new_hash, "successful ACK changed v2")
                require(all(digest(f) == d for f, d in before.items()),
                        "vault or Aquilum witness data was modified")
            finally:
                shutil.rmtree(trial)
    print("AKULA_REAL_APPIMAGE_V1_V2_E2E_PASS real_packages=2 visible_gui=3 restored_v1=1 ack_v2=1 aquilum_unchanged=1", flush=True)


if __name__ == "__main__":
    main()
