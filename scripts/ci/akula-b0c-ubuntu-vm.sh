#!/usr/bin/env bash
set -euo pipefail

# B0c: a separate, disposable Ubuntu 24.04 guest, not a second sshd on the
# runner. VM disk, NoCloud seed, keys, host forward and guest are ephemeral.
# NO customer credentials, real VPS, persistent SSH config or server install.
cd "$(dirname "$0")/../.."
if [[ "$(id -u)" == "0" ]]; then
  echo "B0c refuses to run as the host root account" >&2
  exit 2
fi

for tool in curl sha256sum gpgv ssh ssh-keygen openssl sudo python3; do
  command -v "$tool" >/dev/null
done
if ! command -v qemu-system-x86_64 >/dev/null || ! command -v genisoimage >/dev/null \
    || ! command -v qemu-img >/dev/null \
    || ! test -r /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg; then
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    qemu-system-x86 qemu-utils genisoimage ubuntu-keyring
fi

lab="$(mktemp -d /tmp/akula-b0c-vm.XXXXXXXX)"
chmod 700 "$lab"
guest_pid=""
cleanup() {
  status=$?
  trap - EXIT
  if [[ "$status" != 0 ]] && [[ -f "$lab/console.log" ]]; then
    echo "B0C_EPHEMERAL_GUEST_BOOT_FAILURE_WITNESS" >&2
    tail -n 90 "$lab/console.log" >&2
  fi
  if [[ -n "$guest_pid" ]]; then
    kill "$guest_pid" 2>/dev/null || true
    wait "$guest_pid" 2>/dev/null || true
  fi
  rm -rf -- "$lab"
  exit "$status"
}
trap cleanup EXIT

ssh-keygen -q -t ed25519 -N '' -f "$lab/host_a"
ssh-keygen -q -t ed25519 -N '' -f "$lab/host_b"
ssh-keygen -q -t ed25519 -N '' -f "$lab/client"
ssh-keygen -q -t ed25519 -N '' -f "$lab/wrong_client"
chmod 600 "$lab/host_a" "$lab/host_b" "$lab/client" "$lab/wrong_client"

host_pub="$(cut -d ' ' -f 1,2 "$lab/host_a.pub")"
client_pub="$(cut -d ' ' -f 1,2 "$lab/client.pub")"
random_hash="$(openssl passwd -6 "$(openssl rand -hex 28)")"
mkdir "$lab/seed"
cat > "$lab/seed/meta-data" <<META
instance-id: akula-b0c-disposable-vm-$(openssl rand -hex 8)
local-hostname: akula-b0c
META
cat > "$lab/seed/user-data" <<CLOUD
#cloud-config
hostname: akula-b0c
ssh_deletekeys: true
ssh_genkeytypes: []
ssh_keys:
  ed25519_private: |
$(sed 's/^/    /' "$lab/host_a")
  ed25519_public: $host_pub
ssh_pwauth: false
disable_root: true
users:
  - name: akulavm
    shell: /bin/sh
    lock_passwd: false
    passwd: '$random_hash'
    sudo: null
    ssh_authorized_keys:
      - $client_pub
write_files:
  - path: /etc/ssh/sshd_config.d/90-akula-b0c-no-password.conf
    owner: root:root
    permissions: '0644'
    content: |
      PasswordAuthentication no
      KbdInteractiveAuthentication no
      PermitRootLogin no
      AllowTcpForwarding no
      X11Forwarding no
runcmd:
  - [systemctl, restart, ssh]
CLOUD
unset random_hash
(
  cd "$lab/seed"
  genisoimage -quiet -output "$lab/seed.iso" -volid cidata -rock -joliet \
    user-data meta-data
)

# Validate the Ubuntu cloud image's canonical signed checksum using the
# system-trusted ubuntu-cloudimage-keyring from the apt-managed package.
# This authenticates the disposable VM image, NOT any Akula Node release.
base_url="https://cloud-images.ubuntu.com/noble/current"
image_name="noble-server-cloudimg-amd64.img"
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 \
  "$base_url/SHA256SUMS" -o "$lab/SHA256SUMS"
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 \
  "$base_url/SHA256SUMS.gpg" -o "$lab/SHA256SUMS.gpg"
gpgv --keyring /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg \
  "$lab/SHA256SUMS.gpg" "$lab/SHA256SUMS"
checksum="$(awk '$2 == "*noble-server-cloudimg-amd64.img" || $2 == "noble-server-cloudimg-amd64.img" {print $1; exit}' "$lab/SHA256SUMS")"
if [[ ! "$checksum" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "No validated Ubuntu cloud image checksum" >&2
  exit 1
fi
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 \
  "$base_url/$image_name" -o "$lab/guest.qcow2"
printf '%s  %s\n' "$checksum" "$lab/guest.qcow2" | sha256sum --check --status
qemu-img info "$lab/guest.qcow2" >/dev/null
qemu-img resize "$lab/guest.qcow2" 8G >/dev/null

vm_port="$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"

accelerator=tcg
if [[ -r /dev/kvm && -w /dev/kvm ]]; then accelerator=kvm; fi
echo "B0C_SEPARATE_VM_ACCELERATOR=$accelerator"
qemu-system-x86_64 \
  -machine "accel=$accelerator" -m 2048 -smp 2 \
  -display none -monitor none -serial "file:$lab/console.log" \
  -drive "if=virtio,file=$lab/guest.qcow2,format=qcow2,snapshot=on" \
  -drive "if=virtio,file=$lab/seed.iso,format=raw" \
  -netdev "user,id=labnet,restrict=on,hostfwd=tcp:127.0.0.1:$vm_port-:22" \
  -device virtio-net-pci,netdev=labnet \
  > "$lab/qemu.log" 2>&1 &
guest_pid="$!"

# The independently generated host PUBLIC key was supplied to cloud-init
# BEFORE guest boot. The runner uses only that trusted key here; never TOFU.
printf 'akula-b0c %s\n' "$host_pub" > "$lab/known_hosts"
chmod 600 "$lab/known_hosts"
ready=false
for attempt in $(seq 1 250); do
  if ! kill -0 "$guest_pid" 2>/dev/null; then
    echo "QEMU guest exited before SSH admission" >&2
    tail -n 100 "$lab/qemu.log" >&2
    exit 1
  fi
  if ssh -F /dev/null -T -o BatchMode=yes -o StrictHostKeyChecking=yes \
      -o "UserKnownHostsFile=$lab/known_hosts" \
      -o GlobalKnownHostsFile=/dev/null -o HostKeyAlias=akula-b0c \
      -o HostKeyAlgorithms=ssh-ed25519 -o PasswordAuthentication=no \
      -o KbdInteractiveAuthentication=no -o IdentityAgent=none \
      -o IdentitiesOnly=yes -o ConnectTimeout=3 \
      -o ConnectionAttempts=1 -i "$lab/client" \
      -p "$vm_port" -l akulavm -- 127.0.0.1 true \
      </dev/null >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 2
done
if [[ "$ready" != true ]]; then
  echo "Independent disposable Ubuntu guest was not SSH ready" >&2
  exit 1
fi

# An *independent* OS with its own kernel, filesystem and sshd now serves
# the same six real KEX tests used by B0b localhost test infrastructure.
export AKULA_B0B_SSH_PORT="$vm_port"
export AKULA_B0B_SSH_USER=akulavm
export AKULA_B0B_HOST_A="$host_pub"
export AKULA_B0B_HOST_B="$(cut -d ' ' -f 1,2 "$lab/host_b.pub")"
export AKULA_B0B_CLIENT_KEY="$lab/client"
export AKULA_B0B_WRONG_CLIENT_KEY="$lab/wrong_client"
cd aquilum-app
cargo test --locked -p aquilum-core --features server-bootstrap-b0-openssh-lab \
  --test server_bootstrap_b0b -- --nocapture --test-threads=1
echo "B0C_DISPOSABLE_UBUNTU_VM_REAL_OPENSSH_KEX_SUCCESS"
