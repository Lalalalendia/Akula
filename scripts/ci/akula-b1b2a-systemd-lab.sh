#!/usr/bin/env bash
# B1b2a: ephemeral GitHub Actions host-only systemd confinement experiment.
# No SSH, no release package acceptance, no operator keys, no real vault.
set -euo pipefail
umask 077

if [[ "$GITHUB_ACTIONS" != "true" || "$RUNNER_OS" != "Linux" ]]; then
  echo "B1B2A_REFUSED: ephemeral GitHub Actions Linux runner required" >&2
  exit 2
fi
RUN_ID="$GITHUB_RUN_ID"
ATTEMPT="$GITHUB_RUN_ATTEMPT"
if [[ ! "$RUN_ID" =~ ^[0-9]+$ || ! "$ATTEMPT" =~ ^[0-9]+$ ]]; then
  echo "B1B2A_REFUSED: invalid CI identity" >&2
  exit 2
fi
if [[ ! -d /run/systemd/system ]] || ! command -v systemd-run >/dev/null; then
  echo "B1B2A_REFUSED: real systemd unavailable" >&2
  exit 2
fi
REPO_ROOT="$(git rev-parse --show-toplevel)"
BIN_SRC="$REPO_ROOT/.artifacts/cargo-native/debug/akula-node-lab"
WRAP_SRC="$REPO_ROOT/scripts/ci/akula-b1b2a-node-wrapper.sh"
test -f "$BIN_SRC" && test -f "$WRAP_SRC"

NAME="akula-b1b2a-$RUN_ID-$ATTEMPT"
UNIT="$NAME.service"
BIN_DIR="/opt/$NAME"
VAULT="/var/lib/$NAME-fixture"
SECRET="/run/$NAME-token"
STATE="$NAME-state"
PORT="$((22000 + RUN_ID % 20000))"
LOCAL_TEMP="$(mktemp -d)"
TOKEN_FILE="$LOCAL_TEMP/token"
CURL_CFG="$LOCAL_TEMP/curl.conf"
BODY="$LOCAL_TEMP/response.json"
START_HASH=""
OWN_PATHS=0
UNIT_STARTED=0

cleanup() {
  local code=$?
  trap - EXIT
  if (( UNIT_STARTED == 1 )); then
    if (( code != 0 )); then
      sudo journalctl -u "$UNIT" --no-pager -n 80 2>/dev/null || true
    fi
    sudo systemctl stop "$UNIT" >/dev/null 2>&1 || true
    sudo systemctl reset-failed "$UNIT" >/dev/null 2>&1 || true
  fi
  if (( OWN_PATHS == 1 )); then
    # All paths derive only from numeric CI run identity validated above.
    sudo rm -rf -- "$BIN_DIR" "$VAULT" "/var/lib/$STATE" "/var/lib/private/$STATE"
    sudo rm -f -- "$SECRET"
  fi
  rm -rf -- "$LOCAL_TEMP"
  exit "$code"
}
trap cleanup EXIT

for path in "$BIN_DIR" "$VAULT" "$SECRET" "/var/lib/$STATE" "/var/lib/private/$STATE"; do
  if sudo test -e "$path"; then
    echo "B1B2A_REFUSED: refusing to reuse existing CI installation path" >&2
    exit 2
  fi
done
if ss -lnt "sport = :$PORT" | grep -Fq ":$PORT"; then
  echo "B1B2A_REFUSED: loopback test port in use" >&2
  exit 2
fi

OWN_PATHS=1
TOKEN="$(openssl rand -hex 32)"
printf '%s' "$TOKEN" > "$TOKEN_FILE"
printf 'header = "Authorization: Bearer %s"\n' "$TOKEN" > "$CURL_CFG"
chmod 0600 "$TOKEN_FILE" "$CURL_CFG"
sudo install -d -m 0755 "$BIN_DIR"
sudo install -m 0555 "$BIN_SRC" "$BIN_DIR/akula-node-lab"
sudo install -m 0555 "$WRAP_SRC" "$BIN_DIR/node-wrapper"
sudo install -d -m 0555 "$VAULT"
printf 'b1b2a synthetic read-only fixture\n' > "$LOCAL_TEMP/demo.md"
sudo install -m 0444 "$LOCAL_TEMP/demo.md" "$VAULT/demo.md"
START_HASH="$(sudo sha256sum "$VAULT/demo.md" | cut -d ' ' -f 1)"
sudo install -m 0600 "$TOKEN_FILE" "$SECRET"

sudo systemd-run --unit "$UNIT" \
  --property="Type=simple" \
  --property="DynamicUser=yes" \
  --property="StateDirectory=$STATE" \
  --property="StateDirectoryMode=0700" \
  --property="NoNewPrivileges=yes" \
  --property="ProtectSystem=strict" \
  --property="ProtectHome=yes" \
  --property="PrivateTmp=yes" \
  --property="PrivateDevices=yes" \
  --property="ProtectKernelTunables=yes" \
  --property="ProtectKernelModules=yes" \
  --property="ProtectControlGroups=yes" \
  --property="RestrictAddressFamilies=AF_INET AF_UNIX" \
  --property="IPAddressDeny=any" \
  --property="IPAddressAllow=localhost" \
  --property="ReadOnlyPaths=$VAULT" \
  --property="UMask=0077" \
  --property="Restart=no" \
  --property="LoadCredential=node-token:$SECRET" \
  -- "$BIN_DIR/node-wrapper" "$BIN_DIR/akula-node-lab" "$VAULT" "$PORT"
UNIT_STARTED=1

if [[ "$(sudo systemctl show "$UNIT" --property=DynamicUser --value)" != yes ]] ||
   [[ "$(sudo systemctl show "$UNIT" --property=ProtectSystem --value)" != strict ]]; then
  echo "B1B2A_REFUSED: confinement settings not applied" >&2
  exit 1
fi

URL="http://127.0.0.1:$PORT"
wait_ready() {
  local status
  for _ in $(seq 1 50); do
    status="$(curl --noproxy '*' -s --connect-timeout 1 --max-time 2 \
      --config "$CURL_CFG" -o "$BODY" -w '%{http_code}' "$URL/health" || true)"
    if [[ "$status" == 200 ]] && grep -Fq '"status":"ok"' "$BODY"; then
      return 0
    fi
    sleep 0.2
  done
  echo "B1B2A_REFUSED: sandboxed Node never reached loopback" >&2
  return 1
}
wait_ready

listening="$(ss -lnt "sport = :$PORT")"
if ! grep -Fq "127.0.0.1:$PORT" <<< "$listening" ||
   grep -Eq "(^|[[:space:]])(0\.0\.0\.0|\[::\]):$PORT" <<< "$listening"; then
  echo "B1B2A_REFUSED: service not exclusively bound to IPv4 loopback" >&2
  exit 1
fi

unauth="$(curl --noproxy '*' -s --connect-timeout 1 --max-time 3 \
  -o "$BODY" -w '%{http_code}' "$URL/v1/note" || true)"
if [[ "$unauth" != 401 ]]; then
  echo "B1B2A_REFUSED: unauthenticated read not denied" >&2
  exit 1
fi
approved="$(curl --noproxy '*' -s --connect-timeout 1 --max-time 3 \
  --config "$CURL_CFG" -o "$BODY" -w '%{http_code}' "$URL/v1/note" || true)"
if [[ "$approved" != 200 ]] ||
   ! grep -Fq 'b1b2a synthetic read-only fixture' "$BODY"; then
  echo "B1B2A_REFUSED: scoped synthetic read failed" >&2
  exit 1
fi

# Real independent OS-process death, restart and retained writable state.
sudo systemctl kill --signal=SIGKILL "$UNIT"
sleep 1
sudo systemctl restart "$UNIT"
wait_ready

END_HASH="$(sudo sha256sum "$VAULT/demo.md" | cut -d ' ' -f 1)"
if [[ "$START_HASH" != "$END_HASH" ]]; then
  echo "B1B2A_REFUSED: supposedly read-only fixture was changed" >&2
  exit 1
fi
echo "B1B2A_PASS: separate systemd process, sandbox, loopback, 401, real read, SIGKILL and restart"
