#!/usr/bin/env bash
# Read-only D1 Node, ephemeral VM-only systemd credential wrapper.
set -euo pipefail
test "$#" -eq 3
test -r "$CREDENTIALS_DIRECTORY/node-token"
test -n "$STATE_DIRECTORY"
AKULA_NODE_D1_TOKEN="$(cat "$CREDENTIALS_DIRECTORY/node-token")"
[[ "$AKULA_NODE_D1_TOKEN" =~ ^[0-9a-f]{64}$ ]]
export AKULA_NODE_D1_TOKEN
exec "$1" --vault-root "$2" --appdata "$STATE_DIRECTORY" \
  --note demo.md --port "$3" --max-requests 32
