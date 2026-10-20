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
launched_units=()
cleanup() {
  result=$?
  trap - EXIT
  if [[ "$owned" == true ]]; then
    if (( result != 0 )); then
      sudo journalctl -u "$unit" -o cat -n 80 --no-pager >&2 || true
    fi
    # Every negative test uses a distinct actual transient unit. Stop and
    # reset ALL started test services before deleting their root-owned files.
    for deployed_unit in "${launched_units[@]}"; do
      sudo systemctl stop "$deployed_unit" >/dev/null 2>&1 || true
      sudo systemctl reset-failed "$deployed_unit" >/dev/null 2>&1 || true
    done
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

# A stopped systemd-run transient unit may disappear. Each independent
# negative test MUST launch a NEW, uniquely named UID0 systemd service.
# Never use systemctl start on a discarded transient unit: that can turn
# missing test execution into a false-positive "denied" result.
launch_helper() {
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
  launched_units+=("$unit")
}
launch_helper

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

# An invalid root-staged package MUST cause a real systemd process to
# exit with code 2 and a specific B1a refusal in its own journal.
# Absence of a socket is NOT sufficient: a service that never ran fails CI.
stop_helper() {
  sudo systemctl stop "$unit" >/dev/null 2>&1 || true
  sudo rm -f -- "$sock"
}
expect_refused() {
  local case_name="$1" confirmed=false result="" status=""
  unit="akula-b1b3b-lab-$case_name"
  launch_helper
  for i in $(seq 1 100); do
    if [[ -S "$sock" ]]; then
      echo "B1B3C_REFUSED: unsafe candidate bound a socket ($case_name)" >&2
      return 1
    fi
    result="$(sudo systemctl show "$unit" -p Result --value 2>/dev/null || true)"
    status="$(sudo systemctl show "$unit" -p ExecMainStatus --value 2>/dev/null || true)"
    sudo journalctl -u "$unit" -o cat --no-pager > "$tmp/$case_name.log" 2>/dev/null || true
    if [[ "$result" == exit-code && "$status" == 2 ]] &&
       grep -Fq 'B1B3B_REFUSED: B1a root pinned candidate rejected:' "$tmp/$case_name.log"; then
      confirmed=true
      break
    fi
    sleep 0.1
  done
  if [[ "$confirmed" != true ]]; then
    echo "B1B3C_REFUSED: did not prove actual signed-package rejection ($case_name)" >&2
    sudo journalctl -u "$unit" -o cat -n 60 --no-pager >&2 || true
    return 1
  fi
  echo "B1B3C_SYSTEMD_ACTUAL_PACKAGE_REFUSAL_$case_name"
}
restore_node() {
  sudo install -o root -g root -m 0400 "$tmp/node" "$root_dir/release/node"
}
stop_helper
sudo chmod 0600 "$root_dir/release/node"
sudo sh -c 'printf X >> /run/akula-b1b3b-lab/release/node'
sudo chmod 0400 "$root_dir/release/node"
expect_refused tampered
restore_node
echo "B1B3C_ROOT_TAMPERED_PACKAGE_DENIED"

stop_helper
sudo rm -f -- "$root_dir/release/node"
sudo ln -s manifest "$root_dir/release/node"
expect_refused symlink
sudo rm -f -- "$root_dir/release/node"
restore_node
echo "B1B3C_ROOT_SYMLINK_DENIED"

stop_helper
sudo ln "$root_dir/release/node" "$root_dir/release/hardlink"
expect_refused hardlink
sudo rm -f -- "$root_dir/release/hardlink"
echo "B1B3C_ROOT_HARDLINK_DENIED"

stop_helper
sudo install -o root -g root -m 0600 "$tmp/release-seed" "$root_dir/release.pub"
expect_refused signer
sudo install -o root -g root -m 0600 "$tmp/release.pub" "$root_dir/release.pub"
echo "B1B3C_WRONG_ROOT_SIGNER_DENIED"

stop_helper
unit=akula-b1b3b-lab-restored
launch_helper
for i in $(seq 1 100); do
  [[ -S "$sock" ]] && break
  if ! sudo systemctl is-active --quiet "$unit"; then
    echo "B1B3C_REFUSED: restored signed helper did not start" >&2
    exit 1
  fi
  sleep 0.1
done
[[ -S "$sock" ]]
send 1 "$tmp/valid.frame"
sudo journalctl -u "$unit" -o cat --no-pager > "$tmp/restored.log"
grep -Fq 'B1B3C_ROOT_B1A_SIGNED_FD_PINNED' "$tmp/restored.log"
echo "B1B3C_ROOT_SIGNED_FD_RECOVERY_PASS"
echo "B1B3C_PASS: real systemd denial witnesses + signed Node FD proof; no install authority"
