#!/usr/bin/env bash
# One disposable root-only proof: the EXACT Node-V1 binary built in the same
# CI job is signed by a test-only Ed25519 key and installed using the EXISTING
# B1d0 verified-FD, owner approval and CAS/receipt transaction.
# This does not install or start the production node or authorize a VPS.
set -euo pipefail
umask 077
hold() { printf 'NODE_V1_SIGNED_OPT_HOLD: %s\n' "$*" >&2; exit 1; }

[[ "$GITHUB_ACTIONS" == true && "$RUNNER_OS" == Linux ]] ||
  hold "disposable GitHub Linux runner only"
[[ "$(id -u)" -ne 0 ]] || hold "host workflow must be nonroot"
command -v sudo >/dev/null && command -v unshare >/dev/null ||
  hold "required mount namespace tools unavailable"

ROOT="$(git rev-parse --show-toplevel)"
NODE="$ROOT/interop/akula-staya/target/debug/akula-node-v1-service"
# Resolve the actual Cargo target directory used by the B1d0 test command,
# rather than assuming a checkout-local .cargo/config.toml was loaded.
# Cargo invoked from repository root may use aquilum-app/target instead.
command -v cargo >/dev/null && command -v python3 >/dev/null ||
  hold "Cargo metadata or JSON parser unavailable"
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version=1 \
  --manifest-path "$ROOT/aquilum-app/Cargo.toml" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')" ||
  hold "cannot resolve actual Cargo target directory"
[[ "$TARGET_DIR" = /* && -d "$TARGET_DIR/debug/deps" ]] ||
  hold "resolved Cargo test artifact directory unavailable"
TEST_ROOT="$TARGET_DIR/debug/deps"
MOUNT_ROOT="/var/tmp/akula-b1d0-opt-ci-root"
test -f "$NODE" && test -x "$NODE" || hold "exact Node V1 ELF missing"
test "$(head -c 4 "$NODE" | od -An -tx1 | tr -d ' \n')" = 7f454c46 ||
  hold "unsigned fixture is not a genuine Linux ELF"
test ! -e /opt/akula-node && test ! -L /opt/akula-node ||
  hold "production Node destination already present"
test ! -e /opt/akula-b1d0-ci-node && test ! -L /opt/akula-b1d0-ci-node ||
  hold "existing disposable B1d0 destination"
test ! -e /var/lib/akula/server-bootstrap ||
  hold "production witness already exists"
test ! -e "$MOUNT_ROOT" && test ! -L "$MOUNT_ROOT" ||
  hold "existing disposable mount root"

TEST_BIN=""
count=0
for path in "$TEST_ROOT"/server_bootstrap_b1d0-*; do
  if [[ -f "$path" && -x "$path" && "$path" != *.d ]]; then
    TEST_BIN="$path"
    count=$((count + 1))
  fi
done
[[ "$count" == 1 ]] ||
  hold "expected exactly one compiled B1d0 test in Cargo target/debug/deps (found $count)"

sudo -n install -d -o root -g root -m 0755 "$MOUNT_ROOT"
cleanup() {
  local result=$?
  trap - EXIT
  if ! sudo -n rmdir -- "$MOUNT_ROOT" 2>/dev/null; then
    printf 'NODE_V1_SIGNED_OPT_HOLD: CI mount root not empty; manual review required\n' >&2
    result=1
  fi
  exit "$result"
}
trap cleanup EXIT
test "$(sudo -n stat -c '%u:%a' "$MOUNT_ROOT")" = "0:755" ||
  hold "temporary /opt mount owner/mode invalid"

# /opt is replaced ONLY in this subprocess's mount namespace; the runner
# and any real production destination remain unchanged.
sudo -n unshare --mount --propagation private -- bash -c '
  set -euo pipefail
  mount --bind "$1" /opt
  test "$(stat -c "%u:%a" /opt)" = "0:755"
  test ! -e /opt/akula-node
  test ! -e /opt/akula-b1d0-ci-node
  export GITHUB_ACTIONS=true
  export RUNNER_OS=Linux
  export AKULA_B1D0_CI=STRICT_ROOT_ONE_SHOT_FILE_ONLY
  export AKULA_B1D0_REAL_ELF="$2"
  "$3" --exact b1d0_actual_built_linux_node_elf_uses_one_root_signed_fd_and_opt_receipt --nocapture
  test ! -e /opt/akula-b1d0-ci-node
  test ! -e /opt/akula-node
' node-v1-b1d0 "$MOUNT_ROOT" "$NODE" "$TEST_BIN"

test ! -e /opt/akula-node
test ! -e /opt/akula-b1d0-ci-node
echo "NODE_V1_REAL_SIGNED_ELF_OPT_RECEIPT_AND_CAS_HOLD_PASS"
