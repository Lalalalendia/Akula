#!/usr/bin/env bash
# Official TypeScript SDK -> real disposable Akula Core (direct HTTP, no proxy).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/mcp-direct-sdk"
mkdir -p "$OUT"
export AKAI_MCP_PORT_FILE="$OUT/server-port"
export AKAI_MCP_STOP_FILE="$OUT/stop"
export AKAI_MCP_WORKSPACE_FILE="$OUT/workspace-path"
export AKAI_MCP_SDK_ROOT="$OUT/sdk"
AKAI_MCP_TEST_BEARER=$(python3 -c 'import secrets; print(secrets.token_hex(24))')
export AKAI_MCP_TEST_BEARER
rm -f "$AKAI_MCP_PORT_FILE" "$AKAI_MCP_STOP_FILE" "$AKAI_MCP_WORKSPACE_FILE"

fixture_pid=""
cleanup() {
  touch "$AKAI_MCP_STOP_FILE" || true
  if test -n "$fixture_pid"; then
    wait "$fixture_pid" 2>/dev/null || true
  fi
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
  if test -s "$AKAI_MCP_PORT_FILE" && test -s "$AKAI_MCP_WORKSPACE_FILE"; then break; fi
  if ! kill -0 "$fixture_pid" 2>/dev/null; then
    echo "::error::Disposable Core exited before it was ready"
    tail -n 60 "$OUT/fixture.log"
    exit 1
  fi
  sleep 0.25
done
if ! test -s "$AKAI_MCP_PORT_FILE" || ! test -s "$AKAI_MCP_WORKSPACE_FILE"; then
  echo "::error::Core SDK fixture not ready"
  exit 1
fi

# Existing application CI runner has Node. Pin the official v1 SDK package;
# do not modify the root Cargo or npm dependency graph.
echo "AKAI_DIRECT_SDK_PACKAGE=@modelcontextprotocol/sdk@1.27.1"
node --version
npm --version
timeout --signal=TERM --kill-after=5s 120s npm \
  --prefix "$AKAI_MCP_SDK_ROOT" install \
  --no-save --ignore-scripts --no-audit --no-fund --package-lock=false \
  @modelcontextprotocol/sdk@1.27.1 > "$OUT/npm-install.log" 2>&1

timeout --signal=TERM --kill-after=5s 100s node \
  "$ROOT/scripts/mcp_direct_sdk_smoke.mjs" > "$OUT/direct.log" 2>&1 || {
    echo "::error::Official MCP SDK direct integration failed"
    tail -n 85 "$OUT/direct.log"
    exit 1
  }
cat "$OUT/direct.log"

if test -n "$(printenv GITHUB_STEP_SUMMARY || true)"; then
  {
    echo '### AKAI-CONF-03: direct official MCP SDK + disposable Akula'
    echo 'SDK: @modelcontextprotocol/sdk@1.27.1 (2025 protocol, no proxy)'
    echo 'Read, auth denial and two simultaneous revision-guarded writes:'
    echo '~~~text'
    grep '^AKAI_DIRECT_SDK_RESULT' "$OUT/direct.log"
    echo '~~~'
  } >> "$GITHUB_STEP_SUMMARY"
fi
