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
rm -rf "$OUT/results"
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
# Pin upstream CLI to an immutable release. Full active suite is bounded and
# diagnostic; failures remain visible in raw official checks.json.
set +e
npx --yes @modelcontextprotocol/conformance@0.1.16 server \
  --url "$proxy_url" \
  --scenario server-initialize \
  --spec-version 2025-06-18 \
  --output-dir "$OUT/results/initialize" \
  --verbose > "$OUT/conformance.log" 2>&1
smoke_code=$?

timeout --signal=TERM --kill-after=5s 240s \
  npx --yes @modelcontextprotocol/conformance@0.1.16 server \
    --url "$proxy_url" \
    --suite active \
    --spec-version 2025-06-18 \
    --output-dir "$OUT/results/matrix" \
    --verbose > "$OUT/conformance-matrix.log" 2>&1
matrix_code=$?
set -e

printf 'AKAI_MCP_SMOKE_EXIT=%s\n' "$smoke_code"
printf 'AKAI_MCP_MATRIX_EXIT=%s\n' "$matrix_code"
tail -n 30 "$OUT/conformance.log"
tail -n 90 "$OUT/conformance-matrix.log"

python3 "$ROOT/scripts/mcp_conformance_report.py" \
  --results-root "$OUT/results" \
  --output "$OUT/conformance-summary.json" | tee "$OUT/matrix-summary.log"

if test -n "$(printenv GITHUB_STEP_SUMMARY || true)"; then
  {
    echo '### AKAI-PROT-01 official MCP conformance, revision 2025-06-18'
    echo "Initialize scenario CLI exit: $smoke_code"
    echo "Active scenario matrix CLI exit: $matrix_code (nonzero may mean real nonconformance)"
    echo 'The matrix contains actual FAILED and SUCCESS checks, not an all-pass badge.'
    echo 'Authenticated localhost proxy masks direct auth/SSE/chunked/DNS-rebinding behavior.'
    echo 'Raw checks.json, JSON summary and logs are archived.'
    echo '~~~text'
    cat "$OUT/matrix-summary.log"
    echo '~~~'
  } >> "$GITHUB_STEP_SUMMARY"
fi
