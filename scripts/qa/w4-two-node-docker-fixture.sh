#!/usr/bin/env bash
# CI-only: real OpenSSH and two isolated Linux filesystems / private SQLite
# journals on one runner. No external VPS, Internet delivery or physical PCs.
set -Eeuo pipefail
umask 077

if (( $# != 2 )); then
  echo 'usage: bash w4-two-node-docker-fixture.sh /absolute/Akula.AppImage SHA256' >&2
  exit 2
fi
app=$1
digest=$2
[[ "$app" == /* && -f "$app" && ! -L "$app" && -x "$app" ]] ||
  { echo 'missing / invalid installed AppImage' >&2; exit 2; }
[[ "$digest" =~ ^[0-9a-f]{64}$ ]] ||
  { echo 'expected pinned 64-character SHA256' >&2; exit 2; }
[[ "$(sha256sum "$app" | cut -d' ' -f1)" == "$digest" ]] ||
  { echo 'AppImage SHA256 does not match' >&2; exit 2; }
command -v docker >/dev/null || { echo 'Docker is unavailable' >&2; exit 2; }
command -v ssh-keygen >/dev/null || { echo 'ssh-keygen is unavailable' >&2; exit 2; }
command -v timeout >/dev/null || { echo 'timeout is unavailable' >&2; exit 2; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
tmp="$(mktemp -d /tmp/akula-w4-ssh-fixture.XXXXXXXX)"
suffix="${GITHUB_RUN_ID:-$$}-$$"
container_name="akula-w4-vps-$suffix"
image_name="akula-w4-vps-fixture:$suffix"
cleanup() {
  local code=$?
  trap - EXIT
  docker rm -f "$container_name" >/dev/null 2>&1 || true
  docker image rm -f "$image_name" >/dev/null 2>&1 || true
  rm -rf -- "$tmp"
  if (( code != 0 )); then
    printf 'AKULA_W4_TWO_NAMESPACE_SSH_FAIL exit=%s\n' "$code" >&2
  fi
  exit "$code"
}
trap cleanup EXIT

mkdir -p "$tmp/keys" "$tmp/commands"
ssh-keygen -q -t ed25519 -a 16 -N '' -C w4-ephemeral-client \
  -f "$tmp/keys/client" >/dev/null
ssh-keygen -q -t ed25519 -a 16 -N '' -C w4-ephemeral-vps-host \
  -f "$tmp/keys/host" >/dev/null
chmod 600 "$tmp/keys/client" "$tmp/keys/host"
chmod 644 "$tmp/keys/client.pub" "$tmp/keys/host.pub"
host_pub="$(cat "$tmp/keys/host.pub")"
printf '%s %s\n' 'akula-w4-fixture.invalid' "$host_pub" > "$tmp/keys/known_hosts"
printf '%s %s\n' '[akula-w4-fixture.invalid]:24242' "$host_pub" \
  >> "$tmp/keys/known_hosts"
printf '%s %s\n' '[127.0.0.1]:24242' "$host_pub" \
  >> "$tmp/keys/known_hosts"

cat > "$tmp/keys/ssh-config" <<EOF
Host akula-w4-fixture.invalid
  HostName 127.0.0.1
  Port 24242
  HostKeyAlias akula-w4-fixture.invalid
  UserKnownHostsFile $tmp/keys/known_hosts
  IdentityFile $tmp/keys/client
  IdentitiesOnly yes
  HostKeyAlgorithms ssh-ed25519
  PasswordAuthentication no
  KbdInteractiveAuthentication no
  StrictHostKeyChecking yes
  ForwardAgent no
  ControlMaster no
EOF
chmod 600 "$tmp/keys/ssh-config" "$tmp/keys/known_hosts"

# Keep fixture-specific SSH settings out of the runner's real ~/.ssh.
cat > "$tmp/commands/ssh" <<'EOF'
#!/usr/bin/env bash
exec /usr/bin/ssh -F "$AKULA_W4_FIXTURE_CONFIG" "$@"
EOF
cat > "$tmp/commands/scp" <<'EOF'
#!/usr/bin/env bash
exec /usr/bin/scp -F "$AKULA_W4_FIXTURE_CONFIG" "$@"
EOF
chmod 700 "$tmp/commands/ssh" "$tmp/commands/scp"
export AKULA_W4_FIXTURE_CONFIG="$tmp/keys/ssh-config"
export PATH="$tmp/commands:$PATH"

docker build --pull=false --tag "$image_name" \
  --file "$root/scripts/qa/w4-two-node-vps.Dockerfile" "$root" >/dev/null

# The receiver has its own /etc/machine-id and filesystem and is reachable
# only through a Docker-mapped loopback SSH port on the disposable runner.
docker run --detach --name "$container_name" \
  --publish 127.0.0.1:24242:22 \
  --volume "$tmp/keys/host:/tmp/akula-fixture-host:ro" \
  --volume "$tmp/keys/client.pub:/tmp/akula-fixture-authorized:ro" \
  "$image_name" \
  bash -c 'set -euo pipefail
    install -m 0600 /tmp/akula-fixture-host /etc/ssh/ssh_host_ed25519_key
    install -m 0600 -o w4qa -g w4qa /tmp/akula-fixture-authorized /home/w4qa/.ssh/authorized_keys
    exec /usr/sbin/sshd -D -e' >/dev/null

ready=0
for _ in $(seq 1 100); do
  if /usr/bin/ssh -F "$tmp/keys/ssh-config" -nT -o BatchMode=yes \
    -o StrictHostKeyChecking=yes w4qa@akula-w4-fixture.invalid true \
    >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.25
done
if (( ready != 1 )); then
  docker logs "$container_name" 2>&1 | tail -n 15 >&2 || true
  echo 'real SSH server never became ready' >&2
  exit 1
fi

printf '%s\n' 'AKULA_W4_REAL_SSH_FIXTURE_STARTED encrypted_ssh=1 network_namespace_isolated=1'

# This is the same owner-facing coordinator that will later run against the
# actual VPS, not an alternate fake W4 message protocol.
bash "$root/scripts/qa/w4-two-node-ssh.sh" --preflight \
  "$app" 'w4qa@akula-w4-fixture.invalid' "$digest" |
  grep -Fxq 'AKULA_W4_TWO_NODE_PREFLIGHT_PASS distinct_machine_ids=1 ssh_pinned=1 app_sha256=verified'

report="$tmp/results.txt"
timeout 160 bash "$root/scripts/qa/w4-two-node-ssh.sh" \
  "$app" 'w4qa@akula-w4-fixture.invalid' "$digest" "$report" |
  grep -Fxq 'AKULA_W4_TWO_NODE_SSH_PASS'
test -s "$report"
grep -Fxq 'recipient_signed_w4=verified' "$report"
grep -Fxq 'lost_w4=1' "$report"
grep -Fxq 'duplicate_commits=0' "$report"
grep -Fxq 'physical_two_pc_acceptance=NOT_VERIFIED' "$report"

echo 'AKULA_W4_TWO_NAMESPACE_SSH_PASS ssh_encrypted=1 sealed_W3=1 signed_W4=1 lost_w4=1 duplicate_commits=0'
echo 'AKULA_W4_TEST_LIMITATIONS same_runner=1 public_internet=NOT_TESTED physical_two_pc=NOT_TESTED'
