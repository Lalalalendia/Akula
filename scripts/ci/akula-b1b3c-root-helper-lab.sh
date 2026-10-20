#!/usr/bin/env bash
# Real root/systemd lab service and nonroot peer over AF_UNIX.
# Disposable ubuntu-24.04 GitHub Actions only. NEVER a production installer.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "$(id -u)" == 0 || "$GITHUB_ACTIONS" != true ||
      "$RUNNER_OS" != Linux ]]; then
  echo "B1B3B_REFUSED: disposable unprivileged Linux GitHub runner required" >&2
  exit 2
fi
for executable in sudo systemctl systemd-run python3 openssl; do
  command -v "$executable" >/dev/null
done
root_dir=/run/akula-b1b3b-lab
opt_dir=/opt/akula-b1b3b-lab
sock="$root_dir/helper.sock"
unit=akula-b1b3b-lab
for path in "$root_dir" "$opt_dir"; do
  if [[ -e "$path" || -L "$path" ]]; then
    echo "B1B3B_REFUSED: pre-existing OS path $path" >&2
    exit 2
  fi
done
tmp="$(mktemp -d /tmp/akula-b1b3b-lab.XXXXXXXX)"
chmod 0700 "$tmp"
owned=false
cleanup() {
  result=$?
  trap - EXIT
  if [[ "$owned" == true ]]; then
    if (( result != 0 )); then
      sudo journalctl -u "$unit" -o cat -n 80 --no-pager >&2 || true
    fi
    sudo systemctl stop "$unit" >/dev/null 2>&1 || true
    sudo systemctl reset-failed "$unit" >/dev/null 2>&1 || true
    sudo rm -rf -- "$root_dir" "$opt_dir"
  fi
  rm -rf -- "$tmp"
  exit "$result"
}
trap cleanup EXIT

uid="$(id -u)"
gid="$(id -g)"
test "$uid" -gt 0
openssl rand 32 > "$tmp/seed"
openssl rand 32 > "$tmp/release-seed"
chmod 0600 "$tmp/seed" "$tmp/release-seed"

# Use a REAL ELF node; strip debug info before generating the Ed25519
# signature, so B1a verifies the exact bytes actually staged by root.
cp .artifacts/cargo-native/debug/akula-node-lab "$tmp/node"
strip --strip-debug "$tmp/node"
test -s "$tmp/node"
signer=.artifacts/cargo-native/debug/examples/akula_b1b2b_release_fixture
"$signer" make "$tmp/node" "$tmp/manifest" "$tmp/signature" \
  "$tmp/release.pub" "$tmp/release-seed" 37 0 1 0
"$signer" verify "$tmp/node" "$tmp/manifest" "$tmp/signature" "$tmp/release.pub"

fixture=.artifacts/cargo-native/debug/examples/akula_b1b3c_fixture
binary=.artifacts/cargo-native/debug/akula-helper-b1b3b-lab
test -x "$fixture" && test -x "$binary"
"$fixture" "$tmp" "$tmp/seed" "$uid" "$tmp/manifest" "$tmp/signature" "$tmp/release.pub"

# Root policy is provisioned independently of socket traffic.
# It contains a TEST public key only; no private Ed25519 signing seed.
sudo install -d -o root -g root -m 0755 "$root_dir"
owned=true
sudo install -d -o root -g root -m 0755 "$opt_dir"
printf 'AKULA-B1B3B-CI-MARK\n' > "$tmp/marker"
sudo install -o root -g root -m 0600 "$tmp/marker" "$root_dir/marker"
sudo install -o root -g root -m 0600 "$tmp/policy.bin" "$root_dir/policy.bin"
sudo install -o root -g root -m 0555 "$binary" "$opt_dir/helper"
# Release key is independently provisioned under root-only trust control.
# Candidate package, manifest and signature are *separate* 0400 root files.
sudo install -o root -g root -m 0600 "$tmp/release.pub" "$root_dir/release.pub"
sudo install -d -o root -g root -m 0700 "$root_dir/release"
sudo install -o root -g root -m 0400 "$tmp/node" "$root_dir/release/node"
sudo install -o root -g root -m 0400 "$tmp/manifest" "$root_dir/release/manifest"
sudo install -o root -g root -m 0400 "$tmp/signature" "$root_dir/release/signature"
[[ "$(sudo stat -c '%u:%a' "$root_dir/release/node")" == "0:400" ]]
[[ "$(sudo stat -c '%u:%a' "$root_dir/release.pub")" == "0:600" ]]
[[ "$(sudo stat -c '%u:%a' "$root_dir/policy.bin")" == "0:600" ]]
[[ "$(sudo stat -c '%u:%a' "$root_dir")" == "0:755" ]]
if (printf x >> "$root_dir/policy.bin") 2>/dev/null; then
  echo "B1B3B_REFUSED: nonroot can edit trusted policy" >&2
  exit 1
fi
echo "B1B3B_ROOT_POLICY_UNWRITABLE_BY_PEER"

# Real systemd-managed root process. Restrict network and filesystem writes.
sudo systemd-run --unit="$unit" --service-type=simple \
  --property=User=root \
  --property=NoNewPrivileges=yes \
  --property=ProtectSystem=strict \
  --property=ReadWritePaths="$root_dir" \
  --property=ProtectHome=yes \
  --property=PrivateTmp=yes \
  --property=PrivateDevices=yes \
  --property=RestrictAddressFamilies=AF_UNIX \
  --property=IPAddressDeny=any \
  --property=MemoryMax=128M \
  --property=TimeoutStopSec=5s \
  "$opt_dir/helper"

found=false
for i in $(seq 1 100); do
  if [[ -S "$sock" ]]; then found=true; break; fi
  if ! sudo systemctl is-active --quiet "$unit"; then
    echo "B1B3B_REFUSED: helper service failed before socket bind" >&2
    exit 1
  fi
  sleep 0.1
done
[[ "$found" == true ]]
mainpid="$(sudo systemctl show "$unit" -p MainPID --value)"
[[ "$mainpid" =~ ^[1-9][0-9]*$ ]]
effective_uid="$(sudo awk '/^Uid:/ {print $3}' "/proc/$mainpid/status")"
[[ "$effective_uid" == 0 ]]
[[ "$(sudo systemctl show "$unit" -p ProtectSystem --value)" == strict ]]
[[ "$(sudo systemctl show "$unit" -p NoNewPrivileges --value)" == yes ]]
echo "B1B3B_REAL_ROOT_SYSTEMD_PROCESS_PASS"

# Socket remains root-owned. The test group allows transport access, while
# SO_PEERCRED separately checks kernel-supplied peer UID on every request.
sudo chgrp "$gid" "$sock"
sudo chmod 0660 "$sock"
[[ "$(sudo stat -c '%u:%a' "$sock")" == "0:660" ]]

send() {
  local expected="$1" file="$2" kind=runner
  if [[ $# -ge 3 ]]; then kind="$3"; fi
  if [[ "$kind" == root ]]; then
    sudo python3 - "$sock" "$file" "$expected" <<'PY'
import socket, sys
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
s.settimeout(3)
s.connect(sys.argv[1])
try: s.sendall(open(sys.argv[2],"rb").read())
except BrokenPipeError: pass
try: s.shutdown(socket.SHUT_WR)
except OSError: pass
answer=s.recv(1)
assert answer == bytes([int(sys.argv[3])]), f"unexpected root peer result {answer!r}"
PY
  else
    python3 - "$sock" "$file" "$expected" <<'PY'
import socket, sys
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
s.settimeout(3)
s.connect(sys.argv[1])
s.sendall(open(sys.argv[2],"rb").read())
s.shutdown(socket.SHUT_WR)
answer=s.recv(1)
assert answer == bytes([int(sys.argv[3])]), f"unexpected nonroot peer result {answer!r}"
PY
  fi
}

send 1 "$tmp/valid.frame"
send 0 "$tmp/valid.frame" root
send 0 "$tmp/tampered.frame"
send 0 "$tmp/wrong-host.frame"
send 0 "$tmp/trailing.frame"
send 0 "$tmp/truncated.frame"
send 1 "$tmp/valid.frame"
echo "B1B3B_ROOT_SERVICE_REAL_UID_AND_SIGNED_PLAN_PASS"

sudo journalctl -u "$unit" -o cat --no-pager > "$tmp/journal.log"
grep -Fq "B1B3B_ROOT_SERVICE_BOUND" "$tmp/journal.log"
grep -Fq "B1B3B_PREVIEW_ONLY" "$tmp/journal.log"
grep -Fq "B1B3B_REFUSED" "$tmp/journal.log"
grep -Fq "B1B3C_ROOT_B1A_SIGNED_FD_PINNED" "$tmp/journal.log"
echo "B1B3C_ROOT_SERVICE_B1A_VERIFIED_FD_AND_IPC_PASS"

# A bad root-staged package MUST prevent the helper from binding its socket.
# Destroy only our private root-owned lab path, never outside this namespace.
stop_helper() {
  sudo systemctl stop "$unit" >/dev/null 2>&1 || true
  sudo rm -f -- "$sock"
}
expect_refused() {
  sudo systemctl reset-failed "$unit" >/dev/null 2>&1 || true
  sudo systemctl start "$unit" >/dev/null 2>&1 || true
  sleep 0.4
  if sudo systemctl is-active --quiet "$unit" || [[ -S "$sock" ]]; then
    echo "B1B3C_REFUSED: helper accepted unsafe signed package state" >&2
    exit 1
  fi
}
restore_node() {
  sudo install -o root -g root -m 0400 "$tmp/node" "$root_dir/release/node"
}
stop_helper
sudo chmod 0600 "$root_dir/release/node"
sudo sh -c 'printf X >> /run/akula-b1b3b-lab/release/node'
sudo chmod 0400 "$root_dir/release/node"
expect_refused
restore_node
echo "B1B3C_ROOT_TAMPERED_PACKAGE_DENIED"

stop_helper
sudo rm -f -- "$root_dir/release/node"
sudo ln -s manifest "$root_dir/release/node"
expect_refused
sudo rm -f -- "$root_dir/release/node"
restore_node
echo "B1B3C_ROOT_SYMLINK_DENIED"

stop_helper
sudo ln "$root_dir/release/node" "$root_dir/release/hardlink"
expect_refused
sudo rm -f -- "$root_dir/release/hardlink"
echo "B1B3C_ROOT_HARDLINK_DENIED"

stop_helper
sudo install -o root -g root -m 0600 "$tmp/release-seed" "$root_dir/release.pub"
expect_refused
sudo install -o root -g root -m 0600 "$tmp/release.pub" "$root_dir/release.pub"
echo "B1B3C_WRONG_ROOT_SIGNER_DENIED"

stop_helper
sudo systemctl reset-failed "$unit" >/dev/null 2>&1 || true
sudo systemctl start "$unit"
for i in $(seq 1 100); do
  [[ -S "$sock" ]] && break
  sleep 0.1
done
[[ -S "$sock" ]]
send 1 "$tmp/valid.frame"
echo "B1B3C_ROOT_SIGNED_FD_RECOVERY_PASS"
echo "B1B3C_PASS: signed real Node inode pinned in root helper, policy-bound B1a, rejected tamper/symlink/hardlink/untrusted signer, only inert Preview"
