#!/usr/bin/env bash
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "github-hosted" || "${RUNNER_OS:-}" != "Linux" ]]; then
  echo "Refusing AppImage execution outside an ephemeral GitHub-hosted Linux runner" >&2
  exit 1
fi
shopt -s nullglob
images=("$PWD"/dist/*.AppImage)
[[ "${#images[@]}" == 1 ]] || { echo "Expected exactly one AppImage candidate" >&2; exit 1; }
image="${images[0]}"
[[ "$(basename "$image")" =~ ^akula-app_[0-9]+\.[0-9]+\.[0-9]+_x86_64\.AppImage$ ]] || {
  echo "Unexpected product name for AppImage candidate: $(basename "$image")" >&2; exit 1
}
test -s "$image"
chmod u+x "$image"

# Inspect actual packaged contents, not just an external filename or YAML.
extract="$(mktemp -d)"
container_log="$(mktemp)"
trap 'rm -rf "$extract"; rm -f "$container_log"' EXIT
(
  cd "$extract"
  "$image" --appimage-extract > /dev/null
)
dir="$extract/squashfs-root"
test -x "$dir/AppRun"
test -x "$dir/usr/bin/akula-app"
if find "$dir" \( -name 'Aquilum.desktop' -o -name 'aquilum-app' \) -print | grep -q .; then
  echo "AppImage embeds upstream Aquilum package identity" >&2; exit 1
fi
mapfile -t desktop_files < <(find "$dir" -type f -name '*.desktop')
(( "${#desktop_files[@]}" > 0 )) || { echo "AppImage contains no desktop integration" >&2; exit 1; }
accepted=0
for desktop in "${desktop_files[@]}"; do
  if grep -qx 'Name=Akula' "$desktop" && grep -qE '^Exec=(akula-app|AppRun)([[:space:]]|$)' "$desktop"; then
    accepted=1
    break
  fi
done
[[ "$accepted" == 1 ]] || {
  echo "Extracted AppImage desktop identity does not identify Akula" >&2; exit 1
}
echo 'AKULA_APPIMAGE_STRUCTURE_PASS apprun=1 own_bin=1 own_desktop=1 upstream_paths=0'

# Optional second real version is created only by the isolated signed-staging
# PR job. Its absence in normal beta builds is not an E2E success.
e2e_args=()
container_image=ubuntu:22.04
e2e_lock_digest=""
if [[ -d "$PWD/dist/e2e-v2" ]]; then
  v2="$PWD/dist/e2e-v2/akula-app_0.3.14_x86_64.AppImage"
  helper="$PWD/dist/e2e-v2/transaction-helper"
  [[ -s "$v2" && -x "$helper" ]] || {
    echo 'CI-only 0.3.14 AppImage or real transaction harness missing' >&2
    exit 1
  }
  e2e_args=("/dist/e2e-v2/akula-app_0.3.14_x86_64.AppImage" "/dist/e2e-v2/transaction-helper")
  # Jammy 22.04 has no minisign CLI package. Run this additional
  # signed, two-version GUI experiment on Noble 24.04; regular AppImage
  # smoke still uses Jammy 22.04 unchanged.
  container_image=ubuntu:24.04
  [[ -s "$PWD/dist/e2e-v2/CI_V2_CARGO_LOCK_SHA256.txt" ]] || {
    echo 'CI-only v2 lockfile provenance missing' >&2
    exit 1
  }
  e2e_lock_digest="$(cat "$PWD/dist/e2e-v2/CI_V2_CARGO_LOCK_SHA256.txt")"
  [[ "$e2e_lock_digest" =~ ^[a-f0-9]{64}$ ]] || {
    echo 'CI-only lockfile digest malformed' >&2
    exit 1
  }
fi

# The AppImage is executed in an isolated Docker root FS with isolated
# HOME/XDG_DATA_HOME and only synthetic Aquilum witness data.
# -i is critical: without it, bash -s reads EOF and returns success without
# executing the heredoc, yielding a dangerous false-positive GUI acceptance.
# pipefail preserves container exit failures when output is captured by tee.
docker run --rm -i \
  -e GITHUB_ACTIONS=true -e AKULA_DISPOSABLE_UPDATE_LAB=1 \
  -e AKULA_E2E_SOURCE_COMMIT="${GITHUB_SHA:-}" -e AKULA_E2E_LOCK_SHA256="$e2e_lock_digest" \
  -v "$PWD/dist:/dist:ro" -v "$PWD/scripts:/scripts:ro" \
  "$container_image" bash -s -- "/dist/$(basename "$image")" "${e2e_args[@]}" <<'UBUNTU' | tee "$container_log"
set -euo pipefail
image="$1"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
extra_packages=()
glib_package=libglib2.0-0
if [[ "$#" == 3 ]]; then
  # Ubuntu Noble has minisign and renamed GLib to the t64 package;
  # Ubuntu Jammy baseline retains its original runtime package list.
  extra_packages=(minisign)
  glib_package=libglib2.0-0t64
fi
apt-get install -y -qq --no-install-recommends \
  ca-certificates xvfb openbox wmctrl python3-minimal "${extra_packages[@]}" file "$glib_package" libfontconfig1 libfreetype6 \
  libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
  libxkbcommon0 libxkbcommon-x11-0 libx11-6 libx11-xcb1 \
  libxcursor1 libxrandr2 libxi6 libxcb1 libwayland-client0
mkdir -p /usr/share/applications /tmp/user/.local/share/com.dmitriy.aquilum-app
printf 'upstream binary witness' > /usr/bin/aquilum-app
printf 'upstream desktop witness' > /usr/share/applications/Aquilum.desktop
printf 'upstream appdata witness' > /tmp/user/.local/share/com.dmitriy.aquilum-app/keep
witness_before="$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)"
export HOME=/tmp/user XDG_DATA_HOME=/tmp/user/.local/share DISPLAY=:99
export APPIMAGE_EXTRACT_AND_RUN=1
mkdir -p /tmp/vault
printf '# AppImage acceptance\n\nOriginal synthetic Markdown\n' > /tmp/vault/First.md
original="$(sha256sum /tmp/vault/First.md)"

Xvfb :99 -screen 0 1024x768x24 > /tmp/akula-xvfb.log 2>&1 &
xvfb_pid=$!
sleep 2
openbox > /tmp/akula-wm.log 2>&1 &
wm_pid=$!
app_pid=""
cleanup() {
  if [[ -n "$app_pid" ]]; then kill "$app_pid" 2>/dev/null || true; fi
  kill "$wm_pid" "$xvfb_pid" 2>/dev/null || true
}
trap cleanup EXIT

"$image" --vault /tmp/vault --open /tmp/vault/First.md > /tmp/akula-appimage.log 2>&1 &
app_pid=$!
ready=0
for attempt in $(seq 1 55); do
  kill -0 "$app_pid" 2>/dev/null || {
    echo "AppImage exited before showing Akula window" >&2
    tail -45 /tmp/akula-appimage.log >&2
    exit 1
  }
  if wmctrl -l | grep -q 'Akula'; then ready=1; break; fi
  sleep 1
done
[[ "$ready" == 1 ]] || { echo "AppImage did not create Akula GUI" >&2; tail -45 /tmp/akula-appimage.log; exit 1; }
wmctrl -c Akula
closed=0
for attempt in $(seq 1 45); do
  if ! kill -0 "$app_pid" 2>/dev/null; then closed=1; break; fi
  sleep 0.5
done
[[ "$closed" == 1 ]] || { echo "AppImage GUI refused to exit" >&2; exit 1; }
wait "$app_pid"
app_pid=""
test -f "$XDG_DATA_HOME/io.github.lalalendia.akula/window-state.json"
[[ "$(sha256sum /tmp/vault/First.md)" == "$original" ]] || {
  echo "Synthetic Markdown changed during AppImage GUI acceptance" >&2; exit 1
}
echo 'AKULA_APPIMAGE_GUI_PASS app=Akula isolated_xdg=1 original_note_intact=1'
[[ "$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)" == "$witness_before" ]] || {
  echo "Aquilum witness changed during Akula AppImage acceptance" >&2; exit 1
}
echo 'AKULA_APPIMAGE_COEXIST_PASS upstream_executable=unchanged upstream_desktop=unchanged upstream_appdata=unchanged'

# A SECOND real installed GUI launch tests a generation-bound ACK over the
# inherited Unix stream from the actual AppImage process. The parent verifies
# the process PID, one-time nonce and package hash. No installed file is
# replaced, no update is authorized and no publisher key is used.
ack_log=/tmp/akula-ack-supervisor.log
python3 -u - "$image" > "$ack_log" 2>&1 <<'PYTHON_ACK' &
import hashlib
import os
import secrets
import socket
import subprocess
import sys
from pathlib import Path

image = sys.argv[1]
nonce = secrets.token_hex(16)
digest_state = hashlib.sha256()
with open(image, 'rb') as pkg:
    for block in iter(lambda: pkg.read(1024 * 1024), b''):
        digest_state.update(block)
digest = digest_state.hexdigest()
parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
parent.settimeout(40)
env = dict(os.environ)
env.update({
    "AKULA_LINUX_UPDATE_ACK_FD": str(child.fileno()),
    "AKULA_LINUX_UPDATE_NONCE": nonce,
    "AKULA_LINUX_UPDATE_IMAGE_SHA256": digest,
    "AKULA_LINUX_UPDATE_PARENT_PID": str(os.getpid()),
})
with open("/tmp/akula-appimage-ack-child.log", "wb") as output:
    proc = subprocess.Popen(
        [image, "--vault", "/tmp/vault", "--open", "/tmp/vault/First.md"],
        env=env, pass_fds=(child.fileno(),), stdout=output,
        stderr=subprocess.STDOUT, start_new_session=True,
    )
    child.close()
    try:
        with parent.makefile("rb") as peer:
            line = peer.readline(260)
        # APPIMAGE_EXTRACT_AND_RUN may fork a payload while the runtime
        # waits to clean up extracted files. The GUI is an OS descendant of
        # our launch process, not necessarily identical to the wrapper PID.
        def in_launch_tree(pid, root):
            for _ in range(12):
                if pid == root:
                    return True
                if pid <= 1:
                    return False
                status = Path(f"/proc/{pid}/status").read_text()
                parents = [part.split(":", 1)[1].strip()
                           for part in status.splitlines() if part.startswith("PPid:")]
                if len(parents) != 1:
                    return False
                pid = int(parents[0])
            return False
        parts = line.decode("ascii", "strict").rstrip("\n").split("\t")
        if len(parts) != 4 or parts[0] != "AKULA-LINUX-FIRST-GUI-V1" \
           or parts[1] != nonce or parts[3] != digest:
            raise RuntimeError("ACK protocol/nonce/package digest mismatch: " + repr(line))
        gui_pid = int(parts[2])
        if not in_launch_tree(gui_pid, proc.pid) or os.getpgid(gui_pid) != proc.pid:
            raise RuntimeError(f"ACK sender {gui_pid} is not in AppImage process tree/session {proc.pid}")
        print("AKULA_LINUX_FIRST_GUI_ACK_RECEIVED", flush=True)
        result = proc.wait(timeout=45)
        if result:
            raise RuntimeError(f"actual AppImage GUI exited abnormally: {result}")
        print("AKULA_LINUX_FIRST_GUI_ACK_PASS real_appimage=1 bound_pid=1 nonce=1 digest=1", flush=True)
    except Exception:
        proc.kill()
        proc.wait()
        raise
    finally:
        parent.close()
PYTHON_ACK
ack_supervisor_pid=$!
ack_gui_ready=0
for attempt in $(seq 1 60); do
  if wmctrl -l | grep -q 'Akula'; then ack_gui_ready=1; break; fi
  if ! kill -0 "$ack_supervisor_pid" 2>/dev/null; then break; fi
  sleep 1
done
[[ "$ack_gui_ready" == 1 ]] || {
  echo 'Second AppImage launch never opened a GUI' >&2
  cat "$ack_log" >&2 || true
  tail -60 /tmp/akula-appimage-ack-child.log >&2 || true
  exit 1
}
ack_seen=0
for attempt in $(seq 1 60); do
  if grep -Fxq 'AKULA_LINUX_FIRST_GUI_ACK_RECEIVED' "$ack_log"; then ack_seen=1; break; fi
  if ! kill -0 "$ack_supervisor_pid" 2>/dev/null; then break; fi
  sleep 0.5
done
[[ "$ack_seen" == 1 ]] || {
  echo 'Visible AppImage did not send a verified first-frame ACK' >&2
  cat "$ack_log" >&2 || true
  tail -60 /tmp/akula-appimage-ack-child.log >&2 || true
  exit 1
}
wmctrl -c Akula
wait "$ack_supervisor_pid"
grep -Fxq 'AKULA_LINUX_FIRST_GUI_ACK_PASS real_appimage=1 bound_pid=1 nonce=1 digest=1' "$ack_log"
[[ "$(sha256sum /tmp/vault/First.md)" == "$original" ]] || {
  echo 'First-frame ACK trial mutated the Markdown vault' >&2
  exit 1
}
[[ "$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)" == "$witness_before" ]] || {
  echo 'First-frame ACK trial modified Aquilum witness data' >&2
  exit 1
}
echo 'AKULA_LINUX_FIRST_GUI_ACK_PASS real_appimage=1 bound_pid=1 nonce=1 digest=1'
if [[ "$#" == 3 ]]; then
  python3 -u /scripts/appimage-real-upgrade-accept.py "$1" "$2" "$3"
elif [[ "$#" != 1 ]]; then
  echo 'Unexpected AppImage acceptance argument count' >&2
  exit 1
fi
UBUNTU

# An exit code of zero alone does NOT prove the container executed its script.
# Require both exact markers emitted only after the real OS-level checks.
grep -Fxq 'AKULA_APPIMAGE_GUI_PASS app=Akula isolated_xdg=1 original_note_intact=1' "$container_log"
grep -Fxq 'AKULA_APPIMAGE_COEXIST_PASS upstream_executable=unchanged upstream_desktop=unchanged upstream_appdata=unchanged' "$container_log"
grep -Fxq 'AKULA_LINUX_FIRST_GUI_ACK_PASS real_appimage=1 bound_pid=1 nonce=1 digest=1' "$container_log"
if [[ "${#e2e_args[@]}" -eq 2 ]]; then
  grep -Fxq 'AKULA_REAL_APPIMAGE_V1_V2_E2E_PASS real_packages=2 visible_gui=3 restored_v1=1 ack_v2=1 aquilum_unchanged=1' "$container_log"
fi
echo 'AKULA_APPIMAGE_HOST_GATE_PASS child_script_executed=1 gui_and_coexistence_confirmed=1'
