#!/usr/bin/env bash
# Disposable GitHub-hosted systemd proof of the real Node-V1 binary.
# NOT a signed production installer; never touches a user VPS.
set -euo pipefail
umask 077
refuse() { echo "NODE_V1_SYSTEMD_HOLD: $*" >&2; exit 1; }
[[ "$GITHUB_ACTIONS" == true && "$RUNNER_OS" == Linux ]] ||
  refuse "GitHub-hosted Linux only"
[[ "$(id -u)" -ne 0 && -d /run/systemd/system ]] ||
  refuse "non-root CI with real systemd required"
command -v sudo >/dev/null && command -v ss >/dev/null ||
  refuse "missing systemd OS tools"
ROOT="$(git rev-parse --show-toplevel)"
SERVICE="$ROOT/interop/akula-staya/target/debug/akula-node-v1-service"
ENROLL="/opt/akula-node/node"
UNIT=akula-node-v1.service
STATE=/var/lib/akula-node
OPT=/opt/akula-node
POLICY=/etc/akula-node
UNIT_FILE=/etc/systemd/system/akula-node-v1.service
DROPIN=/etc/systemd/system/akula-node-v1.service.d
USER=akula-node
PORT=43871
test -x "$SERVICE" || refuse "single Node test ELF missing"
for path in "$STATE" "$OPT" "$POLICY" "$UNIT_FILE" "$DROPIN"; do
  [[ ! -e "$path" && ! -L "$path" ]] || refuse "existing OS installation"
done
! getent passwd "$USER" >/dev/null || refuse "service UID already exists"
! getent group "$USER" >/dev/null || refuse "service GID already exists"
! ss -lun | grep -Fq ":$PORT " || refuse "fixed UDP port already busy"
TMP="$(mktemp -d)"
OWN=0
cleanup() {
  local status=$?
  trap - EXIT
  if (( OWN == 1 )); then
    if (( status != 0 )); then
      sudo journalctl -u "$UNIT" --no-pager -n 50 2>/dev/null || true
    fi
    sudo systemctl stop "$UNIT" >/dev/null 2>&1 || true
    sudo systemctl reset-failed "$UNIT" >/dev/null 2>&1 || true
    sudo rm -f -- "$DROPIN/ci-localhost.conf" "$UNIT_FILE"
    sudo rmdir -- "$DROPIN" 2>/dev/null || true
    sudo systemctl daemon-reload >/dev/null 2>&1 || true
    sudo rm -f -- "$POLICY/owner-policy.json" "$OPT/node"
    sudo rmdir -- "$POLICY" "$OPT" 2>/dev/null || true
    sudo rm -f -- "$STATE/node-identity.pk8" "$STATE/node-iroh-seed.bin"
    sudo rm -f -- "$STATE/node-challenges.sqlite" "$STATE/node-challenges.sqlite-journal"
    sudo rmdir -- "$STATE" 2>/dev/null || true
    sudo userdel "$USER" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$TMP"
  exit "$status"
}
trap cleanup EXIT
OWN=1
sudo useradd --system --user-group --no-create-home \
  --home-dir "$STATE" --shell /usr/sbin/nologin "$USER"
sudo install -d -o "$USER" -g "$USER" -m 0700 "$STATE"
sudo install -d -o root -g root -m 0755 "$OPT" "$POLICY" "$DROPIN"
sudo install -o root -g root -m 0555 "$SERVICE" "$OPT/node"
# Both fixed entrypoints live in the exact same root-owned executable.
# No second unverified installation path or target is permitted.
sudo -u "$USER" test -x "$ENROLL" || refuse "Node ELF not service executable"
OUTPUT="$(sudo -u "$USER" "$ENROLL" --enroll-identities)" ||
  refuse "one-time identity enrollment failed"
[[ "$OUTPUT" =~ ^AKULA_NODE_V1_ENROLLED[[:space:]]node=([0-9a-f]{64})[[:space:]]iroh=([0-9a-f]{64})$ ]] ||
  refuse "enrollment did not expose exact public key pair"
NODE_PUBLIC="$(printf '%s\n' "$OUTPUT" | awk '{split($2,a,"=");print a[2]}')"
IROH_PUBLIC="$(printf '%s\n' "$OUTPUT" | awk '{split($3,a,"=");print a[2]}')"
NODE_PUBLIC="$NODE_PUBLIC" IROH_PUBLIC="$IROH_PUBLIC" python3 - "$TMP/policy.json" <<'PY'
import json, os, sys
node = bytes.fromhex(os.environ["NODE_PUBLIC"])
iroh = bytes.fromhex(os.environ["IROH_PUBLIC"])
assert len(node) == len(iroh) == 32 and any(node) and any(iroh)
# Synthetic public anchor: no matching owner private signing key or grants.
config = {
    "version": 3, "node_public": list(node),
    "iroh_endpoint_public": list(iroh), "owner_public": [51] * 32,
    "vault_id": "123e4567-e89b-12d3-a456-426614174000",
    "grant_epoch": 1, "roster_digest": [21] * 32,
    "grant_digest": [31] * 32, "put_grant_digest": [41] * 32,
}
with open(sys.argv[1], "x", encoding="utf-8") as out:
    json.dump(config, out, separators=(",", ":"))
PY
sudo install -o root -g "$USER" -m 0640 "$TMP/policy.json" \
  "$POLICY/owner-policy.json"
# The non-root Node must be able to read the root-owned, non-writable policy.
[[ "$(sudo stat -c '%u:%G:%a:%h' "$POLICY/owner-policy.json")" == "0:$USER:640:1" ]] ||
  refuse "policy is not root-owned, service-readable and immutable"
sudo -u "$USER" test -r "$POLICY/owner-policy.json" ||
  refuse "the non-root service cannot read root-owned policy"
sudo -u "$USER" test ! -w "$POLICY/owner-policy.json" ||
  refuse "CI operator has unexpected write authority over policy"
sudo install -o root -g root -m 0644 \
  "$ROOT/docs/platforms/node-v1/akula-node-v1.service" "$UNIT_FILE"
cat >"$TMP/ci-localhost.conf" <<'EOF'
[Service]
IPAddressDeny=any
IPAddressAllow=localhost
EOF
sudo install -o root -g root -m 0644 \
  "$TMP/ci-localhost.conf" "$DROPIN/ci-localhost.conf"
sudo systemctl daemon-reload
sudo systemctl start "$UNIT"
wait_bound() {
  local i
  for i in $(seq 1 50); do
    if sudo systemctl is-active --quiet "$UNIT" &&
      ss -lun | grep -Fq "0.0.0.0:$PORT "; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}
wait_bound || refuse "actual Iroh daemon did not bind under systemd"
PID1="$(sudo systemctl show "$UNIT" -p MainPID --value)"
[[ "$PID1" =~ ^[1-9][0-9]*$ ]] || refuse "invalid Node PID"
[[ "$(stat -c '%u' "/proc/$PID1")" == "$(id -u "$USER")" ]] ||
  refuse "Node process has incorrect UID"
[[ "$(sudo systemctl show "$UNIT" -p NoNewPrivileges --value)" == yes ]] ||
  refuse "NoNewPrivileges absent"
if sudo -u "$USER" "$ENROLL" --enroll-identities >"$TMP/again.out" 2>"$TMP/again.err"; then
  refuse "enrollment incorrectly permitted identity rotation"
fi
test ! -s "$TMP/again.out" || refuse "failed enrollment returned new identities"
sudo systemctl kill --signal=SIGKILL "$UNIT"
PID2=""
for i in $(seq 1 70); do
  PID2="$(sudo systemctl show "$UNIT" -p MainPID --value)"
  if [[ "$PID2" =~ ^[1-9][0-9]*$ && "$PID2" != "$PID1" ]] &&
    sudo systemctl is-active --quiet "$UNIT" &&
    ss -lun | grep -Fq "0.0.0.0:$PORT "; then
    break
  fi
  sleep 0.2
done
[[ "$PID2" =~ ^[1-9][0-9]*$ && "$PID2" != "$PID1" ]] ||
  refuse "systemd did not restart actual Node process"
for name in node-identity.pk8 node-iroh-seed.bin; do
  [[ "$(sudo stat -c '%a:%h' "$STATE/$name")" == "600:1" ]] ||
    refuse "private key ownership or link changed on restart"
done
# A root-authorized policy revocation must cause a pre-listener HOLD.
sudo systemctl stop "$UNIT"
sudo python3 - "$POLICY/owner-policy.json" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, "r", encoding="utf-8") as source:
    policy = json.load(source)
assert policy["version"] == 3 and len(policy["iroh_endpoint_public"]) == 32
policy["iroh_endpoint_public"][0] ^= 1
with open(path, "w", encoding="utf-8") as destination:
    json.dump(policy, destination, separators=(",", ":"))
PY
sudo systemctl reset-failed "$UNIT" >/dev/null 2>&1 || true
sudo systemctl start "$UNIT" >/dev/null 2>&1 || true
SEEN_HOLD=0
for i in $(seq 1 25); do
  if sudo journalctl -u "$UNIT" --no-pager -o cat -n 40 |
    grep -Fq "Iroh EndpointId is not root-pinned"; then
    SEEN_HOLD=1
    break
  fi
  sleep 0.2
done
[[ "$SEEN_HOLD" == 1 ]] || refuse "revoked public endpoint did not produce a specific HOLD"
if ss -lun | grep -Fq "0.0.0.0:$PORT "; then
  refuse "revoked Iroh identity opened an unauthenticated public socket"
fi
echo "NODE_V1_SYSTEMD_PASS: non-root, protected unit, fixed UDP, SIGKILL restart, preserved keys, revoked endpoint HOLD"
