#!/usr/bin/env bash
set -euo pipefail

# Localhost-only disposable OpenSSH server. GitHub runner only. Never run on
# production or expose a listening port outside 127.0.0.1.
cd "$(dirname "$0")/../.."
if [[ "$(id -u)" == "0" ]]; then
  echo "refuse root test identity" >&2
  exit 2
fi
for tool in ssh ssh-keygen sudo python3 openssl; do
  command -v "$tool" >/dev/null
done
if ! command -v /usr/sbin/sshd >/dev/null; then
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq openssh-server
fi

lab="$(mktemp -d /tmp/akula-b0b-ci.XXXXXXXX)"
chmod 700 "$lab"
sshd_pid=""
user=""
account_created=false
cleanup() {
  status=$?
  trap - EXIT
  # Only ephemeral CI server logs are emitted on failure. Do not enable this
  # for customer servers: their authentication logs may be sensitive.
  if [[ "$status" != 0 ]] && [[ -f "$lab/sshd.log" ]]; then
    echo "B0B_TEST_ONLY_SSHD_FAILURE_DIAGNOSTIC" >&2
    tail -n 90 "$lab/sshd.log" >&2
  fi
  if [[ -n "$sshd_pid" ]]; then
    sudo kill "$sshd_pid" 2>/dev/null || true
    wait "$sshd_pid" 2>/dev/null || true
  fi
  # Do not touch the GitHub runner account. Revoke the entirely synthetic
  # user and its home even after an SSH handshake/CI test failure.
  if [[ "$account_created" == true ]]; then
    sudo userdel -r "$user" 2>/dev/null || true
  fi
  rm -rf -- "$lab"
  exit "$status"
}
trap cleanup EXIT

ssh-keygen -q -N '' -t ed25519 -f "$lab/host_a"
ssh-keygen -q -N '' -t ed25519 -f "$lab/host_b"
ssh-keygen -q -N '' -t ed25519 -f "$lab/client"
ssh-keygen -q -N '' -t ed25519 -f "$lab/wrong_client"
chmod 600 "$lab/client" "$lab/wrong_client" "$lab/host_a" "$lab/host_b"
# Hosted runner accounts may be locked in /etc/shadow, which sshd refuses
# even for valid public-key auth. Use a disposable, non-sudo OS account with
# an unguessable password HASH (password auth is disabled in sshd_config).
# Never unlock, change, or use the real runner login.
user="akulab$(openssl rand -hex 4)"
synthetic_hash="$(openssl passwd -6 "$(openssl rand -hex 32)")"
sudo useradd --create-home --user-group --home-dir "/home/$user" \
  --shell /bin/sh --password "$synthetic_hash" "$user"
account_created=true
unset synthetic_hash
sudo install -d -o "$user" -g "$user" -m 700 "/home/$user/.ssh"
sudo install -o "$user" -g "$user" -m 600 "$lab/client.pub" \
  "/home/$user/.ssh/authorized_keys"

sshport="$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
cat > "$lab/sshd_config" <<CONFIG
Port $sshport
ListenAddress 127.0.0.1
HostKey $lab/host_a
AuthorizedKeysFile .ssh/authorized_keys
PidFile $lab/sshd.pid
HostKeyAlgorithms ssh-ed25519
PubkeyAcceptedAlgorithms ssh-ed25519
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
AllowUsers $user
UsePAM no
StrictModes yes
PermitTTY no
DisableForwarding yes
PermitUserEnvironment no
LogLevel VERBOSE
CONFIG

sudo install -d -m 755 /run/sshd
sudo /usr/sbin/sshd -t -f "$lab/sshd_config"
sudo /usr/sbin/sshd -D -e -f "$lab/sshd_config" > "$lab/sshd.log" 2>&1 &
sshd_pid="$!"

# Only wait for local TCP LISTEN, NOT SSH auth. The Rust integration tests
# prove strict host-key verification using real signed KEX.
for i in $(seq 1 70); do
  if python3 - "$sshport" <<'PY'
import socket,sys
try:
    s=socket.create_connection(("127.0.0.1",int(sys.argv[1])),timeout=0.1)
    s.close()
except OSError:
    sys.exit(1)
PY
  then
    break
  fi
  if [[ "$i" == "70" ]]; then
    cat "$lab/sshd.log" >&2
    exit 1
  fi
  sleep 0.15
done

export AKULA_B0B_SSH_PORT="$sshport"
export AKULA_B0B_SSH_USER="$user"
export AKULA_B0B_HOST_A="$(cut -d ' ' -f 1,2 "$lab/host_a.pub")"
export AKULA_B0B_HOST_B="$(cut -d ' ' -f 1,2 "$lab/host_b.pub")"
export AKULA_B0B_CLIENT_KEY="$lab/client"
export AKULA_B0B_WRONG_CLIENT_KEY="$lab/wrong_client"

cd aquilum-app
cargo test --locked -p aquilum-core --features server-bootstrap-b0-openssh-lab --test server_bootstrap_b0b -- --nocapture --test-threads=1
