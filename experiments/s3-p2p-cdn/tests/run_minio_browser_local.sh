#!/usr/bin/env bash
# Disposable test on a Linux host with Docker, Chromium via Playwright, and Python.
# NOT a WAN/CGNAT/TURN test. Never uses an existing bucket or production credentials.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
for tool in docker curl openssl python3 node; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'MISSING_DEPENDENCY: %s\n' "$tool" >&2
    exit 2
  fi
done
if ! docker info >/dev/null 2>&1; then
  printf 'Docker is not available to this user.\n' >&2
  exit 2
fi
if [[ ! -f "$ROOT/server.py" || ! -f "$ROOT/create_manifest.py" || \
      ! -f "$ROOT/tests/setup_minio_browser.py" || \
      ! -f "$ROOT/tests/test_minio_browser_webrtc.cjs" || \
      ! -d "$ROOT/web" ]]; then
  printf 'Run this script from the complete experiments/s3-p2p-cdn checkout.\n' >&2
  exit 2
fi
if [[ ! -d "$ROOT/node_modules/playwright" ]]; then
  printf 'Install Playwright locally first: cd %s && npm install --no-save playwright@1.56.1 && npx playwright install chromium\n' "$ROOT" >&2
  exit 2
fi
# Ports are intentionally hardcoded in the local-only test modules. Refuse
# to attach to an unknown server that happens to be listening on these ports.
python3 - <<'PY'
import socket
for port in (9000, 9443, 8765):
    with socket.socket() as sock:
        if sock.connect_ex(('127.0.0.1', port)) == 0:
            raise SystemExit(f'PORT_IN_USE: 127.0.0.1:{port}; refusing to use an existing service')
PY

WORK=$(mktemp -d "${TMPDIR:-/tmp}/akula-s3-minio-webrtc.XXXXXXXX")
CONTAINER="akula-s3-minio-browser-$$"
MINIO_IMAGE="${AKULA_MINIO_IMAGE:-cgr.dev/chainguard/minio:latest}"
PYTHON="$(command -v python3)"
EXIT_STATUS=0
GATEWAY_PID=
TRACKER_PID=
TURN_PID=
cleanup() {
  EXIT_STATUS=$?
  trap - EXIT
  # Preserve synthetic evidence before deleting the disposable workspace.
  # GitHub Actions uploads files from the checkout, not from the temp directory.
  EVIDENCE_DIR="$ROOT/test-results/minio-ci"
  mkdir -p "$EVIDENCE_DIR" || EXIT_STATUS=1
  if [[ -d "$WORK/test-results" ]]; then
    cp -a "$WORK/test-results/." "$EVIDENCE_DIR/" || EXIT_STATUS=1
  fi
  for name in server.log minio-gateway.log turn.log; do
    if [[ -f "$WORK/$name" ]]; then
      cp "$WORK/$name" "$EVIDENCE_DIR/$name" || EXIT_STATUS=1
    fi
  done
  for pid in "$GATEWAY_PID" "$TRACKER_PID" "$TURN_PID"; do
    if [[ -n "$pid" ]]; then kill "$pid" >/dev/null 2>&1 || true; fi
  done
  for pid in "$GATEWAY_PID" "$TRACKER_PID"; do
    if [[ -n "$pid" ]]; then wait "$pid" >/dev/null 2>&1 || true; fi
  done
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  if [[ "$EXIT_STATUS" -eq 0 ]]; then
    printf '\nPASS real MinIO-to-browser-to-WebRTC test; disposable resources removed.\n'
    rm -rf -- "$WORK"
  else
    printf '\nFAIL exit=%s; synthetic evidence in: %s; workspace retained at: %s\n' "$EXIT_STATUS" "$EVIDENCE_DIR" "$WORK" >&2
    for log in "$WORK/server.log" "$WORK/minio-gateway.log"; do
      if [[ -f "$log" ]]; then
        printf '\n--- %s (last 40 lines) ---\n' "$log" >&2
        tail -n 40 "$log" >&2 || true
      fi
    done
  fi
  exit "$EXIT_STATUS"
}
trap cleanup EXIT

# A private copy ensures this test never replaces the caller's manifest,
# synthetic fixture, browser data, or existing development configuration.
cp "$ROOT/server.py" "$ROOT/create_manifest.py" "$WORK/"
cp -R "$ROOT/web" "$ROOT/tests" "$WORK/"
ln -s "$ROOT/node_modules" "$WORK/node_modules"
cd "$WORK"

export AWS_ACCESS_KEY_ID=isolated_minio_browser_test_only
export AWS_SECRET_ACCESS_KEY=local_synthetic_password_not_prod_20261024
export AQ_S3_TEST_ENDPOINT=http://127.0.0.1:9000

printf 'Starting throwaway MinIO, bind ONLY to loopback 127.0.0.1:9000...\n'
docker run -d --rm --name "$CONTAINER" \
  -p 127.0.0.1:9000:9000 \
  --tmpfs /data:rw,uid=65532,gid=65532,mode=0700,size=128m \
  -e "MINIO_ROOT_USER=$AWS_ACCESS_KEY_ID" \
  -e "MINIO_ROOT_PASSWORD=$AWS_SECRET_ACCESS_KEY" \
  "$MINIO_IMAGE" server /data >/dev/null
ready=false
for i in $(seq 1 60); do
  if curl --silent --fail --max-time 2 http://127.0.0.1:9000/minio/health/live >/dev/null; then
    ready=true; break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  docker logs "$CONTAINER" >&2 || true
  printf 'MinIO did not become healthy.\n' >&2
  exit 1
fi
printf 'Publishing synthetic 8 MiB object with anonymous read-only GET...\n'
PYTHONPATH=. "$PYTHON" tests/setup_minio_browser.py

# The certificate is ONLY for this local lab, never installed into OS trust.
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout test-minio-key.pem -out test-minio-cert.pem -days 1 \
  -subj '/CN=127.0.0.1' -addext 'subjectAltName=IP:127.0.0.1' >/dev/null 2>&1
"$PYTHON" tests/minio_tls_gateway.py --key test-minio-key.pem --cert test-minio-cert.pem \
  > minio-gateway.log 2>&1 &
GATEWAY_PID=$!
"$PYTHON" server.py > server.log 2>&1 &
TRACKER_PID=$!
ready=false
for i in $(seq 1 60); do
  if curl --silent --fail --max-time 2 http://127.0.0.1:8765/manifest.json >/dev/null && \
     curl --insecure --silent --fail --max-time 2 https://127.0.0.1:9443/metrics >/dev/null; then
    ready=true; break
  fi
  sleep 0.5
done
if [[ "$ready" != true ]]; then
  printf 'Gateway/tracker did not become healthy.\n' >&2
  exit 1
fi
printf 'Running real Chromium P2P and MinIO origin-byte assertions...\n'
node tests/test_minio_browser_webrtc.cjs
test -s test-results/minio-real-browser-result.json
if [[ "${AKULA_TEST_TURN_RELAY:-0}" == "1" ]]; then
  if ! command -v turnserver >/dev/null 2>&1; then
    printf 'MISSING_DEPENDENCY: turnserver (coturn)\n' >&2
    exit 2
  fi
  printf 'Starting test-only local TURN relay...\n'
  turnserver --no-cli --no-tls --no-dtls --no-multicast-peers --no-loopback-peers \
    --listening-ip 127.0.0.1 --relay-ip 127.0.0.1 --listening-port 3478 \
    --min-port 49160 --max-port 49220 --lt-cred-mech \
    --user akula-test:akula-test-only-password --realm akula.test \
    --log-file turn.log --simple-log -v &
  TURN_PID=$!
  sleep 2
  AKULA_TEST_FORCE_RELAY=1 node tests/test_minio_browser_webrtc.cjs
  test -s test-results/minio-turn-browser-result.json
fi
printf '\nOrigin byte counter after test:\n'
curl --insecure --silent --fail https://127.0.0.1:9443/metrics
printf '\n'
