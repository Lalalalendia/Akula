#!/usr/bin/env bash
# E2E-01: two independent Linux hosts over one outbound SSH connection.
# All messaging is via the verified, opt-in Akula AppImage --w4-pilot CLI.
# This experiment does NOT prove two physical PCs, UDP/noq/Roha or K4 routing.
set -Eeuo pipefail
umask 077

usage() {
  cat <<'USAGE'
Usage:
  w4-two-node-ssh.sh --preflight /absolute/Akula.AppImage user@vps.example SHA256
  w4-two-node-ssh.sh /absolute/Akula.AppImage user@vps.example SHA256 [report.txt]
Requirements: Linux x86_64 on both hosts; trusted SSH key already in known_hosts.
The program uploads a COPY of the test AppImage into a new private /tmp
directory on the VPS. It creates disposable test identities and removes them.
It never opens a public Akula port or transfers private signing keys.
USAGE
}
die() { printf 'AKULA_W4_TWO_NODE_FAIL: %s\n' "$*" >&2; exit 1; }

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then usage; exit 0; fi
preflight=0
if [[ "${1:-}" == "--preflight" ]]; then preflight=1; shift; fi
if (( $# != 3 && ( $# != 4 || preflight != 0 ) )); then usage >&2; exit 2; fi

app=$1
destination=$2
expected=$3
report=${4:-}

# Fail closed before starting SSH, creating identities or touching VPS files.
[[ "$app" == /* && -f "$app" && -x "$app" && ! -L "$app" ]] ||
  die "AppImage must be an executable, regular absolute path (not a symlink)"
[[ "$destination" =~ ^[a-z_][a-z0-9_-]*@[A-Za-z0-9][A-Za-z0-9.-]*$ ]] ||
  die "SSH destination must look like non-root-user@hostname; no options"
[[ "${destination%%@*}" != root ]] || die "use a dedicated non-root VPS account"
host_part=${destination#*@}
[[ "$host_part" != localhost && "$host_part" != 127.* && "$host_part" != 0.0.0.0 ]] ||
  die "the recipient must not be a loopback SSH host"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] || die "pass the 64-character pinned SHA256"
if [[ -n "$report" && ( -e "$report" || -L "$report" ) ]]; then
  die "report path exists; refusing to overwrite it"
fi
for program in ssh scp sha256sum timeout mktemp grep sed awk uname; do
  command -v "$program" >/dev/null 2>&1 || die "missing local command: $program"
done
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] ||
  die "this pinned pilot requires Linux x86_64"
actual="$(sha256sum -- "$app" | awk '{print $1}')"
[[ "$actual" == "$expected" ]] || die "local AppImage SHA256 mismatch"
local_id="$(cat /etc/machine-id 2>/dev/null || true)"
[[ "$local_id" =~ ^[0-9a-f]{32}$ ]] ||
  die "local /etc/machine-id unavailable; cannot check host isolation"

ssh_opts=(
  -o BatchMode=yes -o StrictHostKeyChecking=yes
  -o ConnectTimeout=10 -o ServerAliveInterval=10
  -o ServerAliveCountMax=3 -o ExitOnForwardFailure=yes
  -o ForwardAgent=no -o PermitLocalCommand=no
)
remote() { ssh -nT "${ssh_opts[@]}" "$destination" "$1"; }

# Verify the remote operating system and machine identity through an SSH
# session with a previously enrolled host key. No trust-on-first-use.
remote_details="$(remote 'uname -s; uname -m; cat /etc/machine-id' )" ||
  die "SSH preflight failed (check credentials and pinned host key)"
mapfile -t details <<< "$remote_details"
[[ "${details[0]:-}" == Linux && "${details[1]:-}" == x86_64 ]] ||
  die "remote must run Linux x86_64"
[[ "${details[2]:-}" =~ ^[0-9a-f]{32}$ && "${details[2]}" != "$local_id" ]] ||
  die "remote machine identity missing or same as sender"
remote 'command -v mktemp >/dev/null && command -v timeout >/dev/null && command -v sha256sum >/dev/null' ||
  die "remote requires mktemp, timeout and sha256sum"

APPIMAGE_EXTRACT_AND_RUN=1 "$app" --w4-pilot status |
  grep -Fxq 'AKULA_W4_PILOT_READY helper=verified network=inactive auto_start=no SSH_only=yes' ||
  die "local W4 helper failed its embedded SHA256 check"

if (( preflight )); then
  printf 'AKULA_W4_TWO_NODE_PREFLIGHT_PASS distinct_machine_ids=1 ssh_pinned=1 app_sha256=verified\n'
  exit 0
fi

local_dir="$(mktemp -d /tmp/akula-w4-sender.XXXXXXXX)"
remote_dir=''
receiver_pid=''
tunnel_pid=''
cleanup() {
  local result=$?
  trap - EXIT
  if [[ -n "$receiver_pid" ]]; then
    kill "$receiver_pid" 2>/dev/null || true
    wait "$receiver_pid" 2>/dev/null || true
  fi
  if [[ -n "$tunnel_pid" ]]; then
    kill "$tunnel_pid" 2>/dev/null || true
    wait "$tunnel_pid" 2>/dev/null || true
  fi
  if [[ -n "$remote_dir" ]]; then
    remote "rm -rf -- '$remote_dir'" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$local_dir"
  if (( result != 0 )); then
    printf 'AKULA_W4_TWO_NODE_RESULT=FAIL (no user Vault was modified)\n' >&2
  fi
  return "$result"
}
trap cleanup EXIT

remote_dir="$(remote 'umask 077; mktemp -d /tmp/akula-w4-remote.XXXXXXXX')" ||
  die "remote scratch directory creation failed"
[[ "$remote_dir" =~ ^/tmp/akula-w4-remote\.[A-Za-z0-9]{8}$ ]] ||
  die "unsafe remote scratch path; refusing to use it"

scp -q "${ssh_opts[@]}" "$app" "$destination:$remote_dir/akula.AppImage" ||
  die "could not upload synthetic test AppImage to remote scratch directory"
remote "chmod 700 '$remote_dir/akula.AppImage'" ||
  die "could not protect remote binary"
remote_digest="$(remote "sha256sum '$remote_dir/akula.AppImage' | awk '{print \$1}'")" ||
  die "remote SHA256 failed"
[[ "$remote_digest" == "$expected" ]] || die "remote executable changed in transit"
remote "APPIMAGE_EXTRACT_AND_RUN=1 '$remote_dir/akula.AppImage' --w4-pilot status" |
  grep -Fxq 'AKULA_W4_PILOT_READY helper=verified network=inactive auto_start=no SSH_only=yes' ||
  die "remote W4 helper failed its embedded SHA256 check"

local_init="$(APPIMAGE_EXTRACT_AND_RUN=1 XDG_DATA_HOME="$local_dir/data" "$app" --w4-pilot init-sender)" ||
  die "sender key generation failed"
remote_init="$(remote "APPIMAGE_EXTRACT_AND_RUN=1 XDG_DATA_HOME='$remote_dir/data' '$remote_dir/akula.AppImage' --w4-pilot init-recipient")" ||
  die "recipient key generation failed"
field() {
  local match
  match="$(printf '%s\n' "$3" | grep -E "^$1=[0-9a-f]{$2}$" || true)"
  [[ -n "$match" && "$match" != *$'\n'* ]] || die "missing or duplicate public field $1"
  printf '%s' "${match#*=}"
}
sender_key="$(field SENDER_VERIFY 64 "$local_init")"
invite="$(field RECIPIENT_INVITE 160 "$remote_init")"

# Only the sender makes an outgoing SSH connection. On the VPS, Akula's
# single-use receiver listens on 127.0.0.1:42042; no application WAN port.
ssh -nNT "${ssh_opts[@]}" -L 127.0.0.1:42043:127.0.0.1:42042 \
  "$destination" >"$local_dir/tunnel.log" 2>&1 &
tunnel_pid=$!
sleep 1
kill -0 "$tunnel_pid" 2>/dev/null ||
  die "SSH tunnel could not bind local loopback port 42043"

start_receiver() {
  local action=$1 log=$2 command i
  [[ "$action" == listen || "$action" == listen-drop-ack ]] ||
    die "invalid receiver action"
  command="APPIMAGE_EXTRACT_AND_RUN=1 XDG_DATA_HOME='$remote_dir/data' timeout 50 '$remote_dir/akula.AppImage' --w4-pilot "
  if [[ "$action" == listen ]]; then
    command+="listen '$sender_key'"
  else
    command+="listen-drop-ack '$sender_key'"
  fi
  remote "$command" > "$log" 2>&1 &
  receiver_pid=$!
  for ((i=0;i<75;i++)); do
    if grep -Fxq 'READY 127.0.0.1:42042' "$log"; then return 0; fi
    kill -0 "$receiver_pid" 2>/dev/null ||
      die "recipient exited before binding its loopback port"
    sleep 0.2
  done
  die "recipient did not become ready"
}
finish_receiver() {
  wait "$receiver_pid" || die "remote recipient returned failure"
  receiver_pid=''
}
local_send() {
  timeout 50 env APPIMAGE_EXTRACT_AND_RUN=1 XDG_DATA_HOME="$local_dir/data" \
    "$app" --w4-pilot "$@"
}

start_receiver listen "$local_dir/first-receiver.log"
local_send send-test "$invite" > "$local_dir/first-sender.log" 2>&1 ||
  die "initial W3 message did not receive its signed W4"
finish_receiver
first_id="$(field MESSAGE_ID 32 "$(cat "$local_dir/first-sender.log")")"
grep -Fxq "RECIPIENT_COMMITTED $first_id new" "$local_dir/first-receiver.log" ||
  die "recipient failed to durably insert first W3"
grep -Fxq "W4_SENT $first_id" "$local_dir/first-receiver.log" ||
  die "recipient did not sign/send first W4"
grep -Fxq "RECIPIENT_CONFIRMED $first_id" "$local_dir/first-sender.log" ||
  die "sender did not verify first signed W4"

start_receiver listen-drop-ack "$local_dir/drop-receiver.log"
if local_send send-test "$invite" > "$local_dir/drop-sender.log" 2>&1; then
  die "sender incorrectly accepted a missing W4"
fi
finish_receiver
dropped_id="$(field MESSAGE_ID 32 "$(cat "$local_dir/drop-sender.log")")"
grep -Fxq "RECIPIENT_COMMITTED $dropped_id new" "$local_dir/drop-receiver.log" ||
  die "recipient failed to commit W3 before dropping W4"
grep -Fq 'INJECTED_W4_DROP' "$local_dir/drop-receiver.log" ||
  die "expected W4 loss was not injected"

start_receiver listen "$local_dir/retry-receiver.log"
local_send resend "$invite" "$dropped_id" > "$local_dir/retry-sender.log" 2>&1 ||
  die "exact retry failed"
finish_receiver
grep -Fxq "RECIPIENT_COMMITTED $dropped_id duplicate" "$local_dir/retry-receiver.log" ||
  die "recipient incorrectly inserted duplicate W3"
grep -Fxq "RECIPIENT_CONFIRMED $dropped_id" "$local_dir/retry-sender.log" ||
  die "replayed W3 did not receive its signed W4"

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
id_hash="$(printf %s "$dropped_id" | sha256sum | awk '{print $1}')"
result="AKULA_W4_TWO_NODE_SSH_PASS
timestamp_utc=$timestamp
appimage_sha256=$expected
message_id_sha256=$id_hash
distinct_linux_machine_ids=1
ssh_host_key_mode=strict_pinned
application_listener=127.0.0.1:42042
bytes=1024
recipient_signed_w4=verified
lost_w4=1
duplicate_commits=0
physical_two_pc_acceptance=NOT_VERIFIED
roha_noq_udp_acceptance=NOT_VERIFIED"
printf '%s\n' "$result"
if [[ -n "$report" ]]; then
  (set -C; printf '%s\n' "$result" > "$report") || die "could not create protected evidence report"
fi
