#!/usr/bin/env bash
# B1b2a disposable systemd test wrapper. Never installed on a user server.
set -euo pipefail
test "$#" -eq 3
test -n "$CREDENTIALS_DIRECTORY"
test -n "$STATE_DIRECTORY"
test -r "$CREDENTIALS_DIRECTORY/node-token"
AKULA_NODE_D1_TOKEN="$(cat "$CREDENTIALS_DIRECTORY/node-token")"
test "${#AKULA_NODE_D1_TOKEN}" -eq 64
export AKULA_NODE_D1_TOKEN
exec "$1" --vault-root "$2" --appdata "$STATE_DIRECTORY" --note demo.md --port "$3" --max-requests 24
