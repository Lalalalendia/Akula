#!/usr/bin/env bash
# No credentials, real SSH target, or live network are needed for these tests.
set -Eeuo pipefail
script_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
runner="$script_dir/w4-two-node-ssh.sh"
sandbox="$(mktemp -d /tmp/akula-w4-shell-test.XXXXXXXX)"
trap 'rm -rf -- "$sandbox"' EXIT
mkdir -p "$sandbox/bin"
cat > "$sandbox/app.AppImage" <<'SH'
#!/usr/bin/env bash
if [[ "$#" == 2 && "$1" == --w4-pilot && "$2" == status ]]; then
  echo 'AKULA_W4_PILOT_READY helper=verified network=inactive auto_start=no SSH_only=yes'
else
  exit 42
fi
SH
chmod 700 "$sandbox/app.AppImage"
sha="$(sha256sum "$sandbox/app.AppImage" | awk '{print $1}')"

cat > "$sandbox/bin/ssh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$AKULA_MOCK_CALLS"
if [[ "${AKULA_MOCK_MODE:-}" == refuse ]]; then exit 255; fi
if [[ "$*" == *"uname -s; uname -m; cat /etc/machine-id"* ]]; then
  printf 'Linux\nx86_64\n%s\n' "${AKULA_MOCK_REMOTE_ID:-00000000000000000000000000000000}"
fi
SH
chmod 700 "$sandbox/bin/ssh"
export PATH="$sandbox/bin:$PATH" AKULA_MOCK_CALLS="$sandbox/calls"

bash -n "$runner"
bash "$runner" --help | grep -q 'E2E' || bash "$runner" --help | grep -q 'two' # help output is stable

# Inputs must be rejected *before* any SSH invocation.
: > "$sandbox/calls"
if bash "$runner" "$sandbox/app.AppImage" '-oProxyCommand=evil' "$sha" >/dev/null 2>&1; then exit 1; fi
if bash "$runner" "$sandbox/app.AppImage" 'root@vps.example' "$sha" >/dev/null 2>&1; then exit 1; fi
if bash "$runner" "$sandbox/app.AppImage" 'user@localhost' "$sha" >/dev/null 2>&1; then exit 1; fi
if bash "$runner" "$sandbox/app.AppImage" 'user@vps.example' 'invalidsha' >/dev/null 2>&1; then exit 1; fi
fake_sha="$(printf '%064d' 0)"
if [[ "$sha" == "$fake_sha" ]]; then fake_sha="$(printf '%064d' 1)"; fi
if bash "$runner" "$sandbox/app.AppImage" 'user@vps.example' "$fake_sha" >/dev/null 2>&1; then exit 1; fi
if [[ -s "$sandbox/calls" ]]; then
  echo "SSH was invoked before validation" >&2
  exit 1
fi

# The positive preflight must require key pinning and no password prompts.
export AKULA_MOCK_REMOTE_ID=ffffffffffffffffffffffffffffffff
local_id="$(cat /etc/machine-id)"
if [[ "$local_id" == "$AKULA_MOCK_REMOTE_ID" ]]; then
  export AKULA_MOCK_REMOTE_ID=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
fi
bash "$runner" --preflight "$sandbox/app.AppImage" 'user@vps.example' "$sha" \
  > "$sandbox/good"
grep -q '^AKULA_W4_TWO_NODE_PREFLIGHT_PASS ' "$sandbox/good"
grep -Fq 'StrictHostKeyChecking=yes' "$sandbox/calls"
grep -Fq 'BatchMode=yes' "$sandbox/calls"
grep -Fq 'ForwardAgent=no' "$sandbox/calls"
if grep -Fq 'StrictHostKeyChecking=no' "$sandbox/calls"; then exit 1; fi

# Same machine ID and rejected SSH must prevent any experiment.
export AKULA_MOCK_REMOTE_ID="$local_id"
if bash "$runner" --preflight "$sandbox/app.AppImage" 'user@vps.example' "$sha" > /dev/null 2>&1; then
  echo "two accounts on the same machine were accepted" >&2
  exit 1
fi
export AKULA_MOCK_MODE=refuse
if bash "$runner" --preflight "$sandbox/app.AppImage" 'user@vps.example' "$sha" > /dev/null 2>&1; then
  echo "rejected SSH host key was accepted" >&2
  exit 1
fi
echo 'AKULA_W4_TWO_NODE_SCRIPT_TEST_PASS pinned_ssh=1 fail_closed=1 secret_transfer=0'
