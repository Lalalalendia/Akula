#!/usr/bin/env bash
# Official upstream conformance vs real disposable Akula Core (diagnostic only).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/mcp-conformance-smoke"
mkdir -p "$OUT"
export AKAI_MCP_PORT_FILE="$OUT/server-port"
export AKAI_MCP_STOP_FILE="$OUT/stop"
AKAI_MCP_TEST_BEARER=$(python3 -c 'import secrets; print(secrets.token_hex(24))')
export AKAI_MCP_TEST_BEARER
rm -f "$AKAI_MCP_PORT_FILE" "$AKAI_MCP_STOP_FILE" "$OUT/proxy-port"
fixture_pid=""
proxy_pid=""
cleanup() {
  touch "$AKAI_MCP_STOP_FILE" || true
  if test -n "$proxy_pid"; then kill "$proxy_pid" 2>/dev/null || true; fi
  if test -n "$fixture_pid"; then wait "$fixture_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT

(
  cd "$ROOT/aquilum-app"
  cargo test --locked -p aquilum-core --lib \
    isolated_conformance_fixture_serves_temporary_core \
    -- --ignored --nocapture --test-threads=1
) > "$OUT/fixture.log" 2>&1 &
fixture_pid=$!

for i in $(seq 1 120); do
  test -s "$AKAI_MCP_PORT_FILE" && break
  if ! kill -0 "$fixture_pid" 2>/dev/null; then
    echo "::error::Temporary MCP Core fixture exited before binding"
    tail -n 45 "$OUT/fixture.log"
    exit 1
  fi
  sleep 0.25
done
test -s "$AKAI_MCP_PORT_FILE" || { echo "::error::MCP fixture timeout"; exit 1; }

python3 "$ROOT/scripts/mcp_conformance_proxy.py" \
  --upstream-port-file "$AKAI_MCP_PORT_FILE" \
  --listen-port-file "$OUT/proxy-port" > "$OUT/proxy.log" 2>&1 &
proxy_pid=$!
for i in $(seq 1 80); do
  test -s "$OUT/proxy-port" && break
  if ! kill -0 "$proxy_pid" 2>/dev/null; then
    echo "::error::Temporary MCP Bearer adapter exited"
    tail -n 45 "$OUT/proxy.log"
    exit 1
  fi
  sleep 0.25
done
test -s "$OUT/proxy-port" || { echo "::error::MCP test adapter timeout"; exit 1; }

proxy_url="http://127.0.0.1:$(cat "$OUT/proxy-port")/mcp"
# Pin the upstream CLI. Record failure, do not fabricate a passing verdict.
set +e
npx --yes @modelcontextprotocol/conformance@0.1.16 server \
  --url "$proxy_url" \
  --scenario server-initialize \
  --spec-version 2025-06-18 \
  --output-dir "$OUT/results" \
  --verbose > "$OUT/conformance.log" 2>&1
suite_code=$?
set -e
printf 'AKAI_MCP_CONFORMANCE_RUNNER_EXIT=%s\n' "$suite_code"
tail -n 65 "$OUT/conformance.log"

if ! find "$OUT/results" -name checks.json -type f 2>/dev/null | grep -q .; then
  echo "::error::Official conformance runner produced no checks.json"
  exit 1
fi
if test -n "$(printenv GITHUB_STEP_SUMMARY || true)"; then
  {
    echo '### AKAI-PROT-01 official MCP conformance smoke'
    echo 'Version: 2025-06-18 / server-initialize / CLI 0.1.16'
    echo "Conformance CLI exit code: $suite_code"
    echo 'Bearer adapter: loopback test-only, not full transport or authorization conformance.'
    echo 'Raw checks.json and logs are in the CI artifact.'
  } >> "$GITHUB_STEP_SUMMARY"
fi
