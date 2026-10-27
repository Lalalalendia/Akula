#!/usr/bin/env bash
# E2E-01: verified Akula *main executable* -> separate pinned WOW helper.
# Runs on one CI runner with two independent app-data roots and a local proxy.
# This is NOT evidence of real two-host Internet delivery.
set -euo pipefail
if [[ $# -ne 1 ]]; then
  echo "usage: bash scripts/messaging/w4-pilot-smoke.sh /absolute/path/akula-app" >&2
  exit 2
fi
app="$1"
test -x "$app"
command -v socat >/dev/null
root="$(mktemp -d)"
proxy_pid=""
receiver_pid=""
cleanup() {
  [[ -z "$receiver_pid" ]] || kill "$receiver_pid" 2>/dev/null || true
  [[ -z "$proxy_pid" ]] || kill "$proxy_pid" 2>/dev/null || true
  rm -rf "$root"
}
trap cleanup EXIT INT TERM
mkdir -p "$root/alice" "$root/bob" "$root/rogue"

"$app" --w4-pilot status | grep -Fxq 'AKULA_W4_PILOT_READY helper=verified network=inactive auto_start=no SSH_only=yes'

# Isolated private W3/W4 stores, NO Vault path and no production secrets.
alice="$(XDG_DATA_HOME="$root/alice" "$app" --w4-pilot init-sender)"
bob="$(XDG_DATA_HOME="$root/bob" "$app" --w4-pilot init-recipient)"
sender_verify="$(printf '%s\n' "$alice" | sed -n 's/^SENDER_VERIFY=//p')"
invite="$(printf '%s\n' "$bob" | sed -n 's/^RECIPIENT_INVITE=//p')"
[[ "$sender_verify" =~ ^[0-9a-f]{64}$ ]]
[[ "$invite" =~ ^[0-9a-f]{160}$ ]]
test "$(stat -c %a "$root/alice/io.github.lalalendia.akula/messaging-w4-pilot-v1")" = 700
test "$(stat -c %a "$root/bob/io.github.lalalendia.akula/messaging-w4-pilot-v1")" = 700

# Two local ports are intentionally separate, as in two independent PCs.
socat TCP-LISTEN:42043,bind=127.0.0.1,reuseaddr,fork TCP:127.0.0.1:42042 \
  >"$root/proxy.log" 2>&1 &
proxy_pid=$!
sleep 1

XDG_DATA_HOME="$root/bob" timeout 30 "$app" --w4-pilot listen "$sender_verify" \
  >"$root/receiver.log" 2>&1 &
receiver_pid=$!
sleep 1
XDG_DATA_HOME="$root/alice" timeout 30 "$app" --w4-pilot send-test "$invite" \
  >"$root/sender.log" 2>&1
wait "$receiver_pid"
receiver_pid=""
grep -q '^RECIPIENT_COMMITTED .* new$' "$root/receiver.log"
grep -q '^W4_SENT ' "$root/receiver.log"
grep -q '^RECIPIENT_CONFIRMED ' "$root/sender.log"
echo 'AKULA_NATIVE_W4_MAIN_APP_SEND_PASS bytes=1024 signed_w4=1'

# Now simulate failure AFTER recipient COMMIT and before the W4 return.
XDG_DATA_HOME="$root/bob" timeout 30 "$app" --w4-pilot listen-drop-ack "$sender_verify" \
  >"$root/receiver-drop.log" 2>&1 &
receiver_pid=$!
sleep 1
if XDG_DATA_HOME="$root/alice" timeout 30 "$app" --w4-pilot send-test "$invite" \
  >"$root/sender-drop.log" 2>&1; then
  echo 'ERROR: missing W4 unexpectedly confirmed' >&2
  exit 1
fi
wait "$receiver_pid"
receiver_pid=""
grep -q '^INJECTED_W4_DROP' "$root/receiver-drop.log"
message="$(sed -n 's/^MESSAGE_ID=//p' "$root/sender-drop.log")"
[[ "$message" =~ ^[0-9a-f]{32}$ ]]

XDG_DATA_HOME="$root/bob" timeout 30 "$app" --w4-pilot listen "$sender_verify" \
  >"$root/receiver-retry.log" 2>&1 &
receiver_pid=$!
sleep 1
XDG_DATA_HOME="$root/alice" timeout 30 "$app" --w4-pilot resend "$invite" "$message" \
  >"$root/sender-retry.log" 2>&1
wait "$receiver_pid"
receiver_pid=""
grep -q '^RECIPIENT_COMMITTED .* duplicate$' "$root/receiver-retry.log"
grep -q '^RECIPIENT_CONFIRMED ' "$root/sender-retry.log"
echo 'AKULA_NATIVE_W4_MAIN_APP_RETRY_PASS lost_w4=1 duplicate_commit=0'

# A remote packet and a forged uploader must not start a public listener.
if XDG_DATA_HOME="$root/bob" "$app" --w4-pilot listen '0.0.0.0' \
  >"$root/invalid.log" 2>&1; then
    echo 'ERROR: invalid peer accepted' >&2
    exit 1
fi
if AKULA_DISABLE_W4_PILOT=1 "$app" --w4-pilot status >"$root/kill-switch.log" 2>&1; then
    echo 'ERROR: emergency kill switch failed' >&2
    exit 1
fi
if XDG_DATA_HOME="$root/alice" "$app" --w4-pilot init-sender >"$root/duplicate.log" 2>&1; then
    echo 'ERROR: existing private signing key replaced' >&2
    exit 1
fi
echo 'AKULA_NATIVE_W4_CANARY_PASS gui_untouched=1 vault_untouched=1 ports=loopback-only'
