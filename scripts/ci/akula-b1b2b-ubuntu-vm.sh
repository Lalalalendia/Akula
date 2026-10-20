#!/usr/bin/env bash
# B1b2b disposable Ubuntu QEMU signed Node upgrade/rollback lab.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "$(id -u)" == 0 || "$GITHUB_ACTIONS" != true || "$RUNNER_OS" != Linux ]]; then
  echo "B1B2B_REFUSED: CI-only unprivileged runner" >&2
  exit 2
fi
for cmd in curl sha256sum gpgv ssh scp ssh-keygen openssl python3 sudo strip; do
  command -v "$cmd" >/dev/null
done
if ! command -v qemu-system-x86_64 >/dev/null ||
   ! command -v genisoimage >/dev/null ||
   ! command -v qemu-img >/dev/null ||
   ! test -r /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg; then
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    qemu-system-x86 qemu-utils genisoimage ubuntu-keyring
fi
test -f .artifacts/cargo-native/debug/akula-node-lab
test -x .artifacts/cargo-native/debug/examples/akula_b1b2b_release_fixture
lab="$(mktemp -d /tmp/akula-b1b2b-ubuntu.XXXXXXXX)"
chmod 0700 "$lab"
guest_pid=""
cleanup(){
  result=$?
  trap - EXIT
  if (( result != 0 )); then
    echo "B1B2B_VM_FAILURE_WITNESS" >&2
    test ! -f "$lab/console.log" || tail -n 80 "$lab/console.log" >&2
    test ! -f "$lab/qemu.log" || tail -n 30 "$lab/qemu.log" >&2
  fi
  if [[ -n "$guest_pid" ]]; then
    kill "$guest_pid" 2>/dev/null || true
    wait "$guest_pid" 2>/dev/null || true
  fi
  rm -rf -- "$lab"
  exit "$result"
}
trap cleanup EXIT
ssh-keygen -q -t ed25519 -N '' -f "$lab/host"
ssh-keygen -q -t ed25519 -N '' -f "$lab/client"
chmod 0600 "$lab/host" "$lab/client"
host_pub="$(cut -d ' ' -f 1,2 "$lab/host.pub")"
client_pub="$(cut -d ' ' -f 1,2 "$lab/client.pub")"
openssl rand 32 > "$lab/test-signer-seed"
openssl rand -hex 32 > "$lab/node-token"
chmod 0600 "$lab/test-signer-seed" "$lab/node-token"
cp .artifacts/cargo-native/debug/akula-node-lab "$lab/v1.node"
# Strip debug symbols BEFORE signing to avoid sending a ~176 MB debug ELF over
# software-emulated QEMU/SSH; B1a authenticates the exact stripped bytes.
strip --strip-debug "$lab/v1.node"
test -s "$lab/v1.node"
echo "B1B2B_SIGNED_ELF_BYTES=$(wc -c < "$lab/v1.node")"
printf '#!/bin/sh\nexit 42\n' > "$lab/v2.node"
chmod 0555 "$lab/v2.node"
signer=.artifacts/cargo-native/debug/examples/akula_b1b2b_release_fixture
"$signer" make "$lab/v1.node" "$lab/v1.manifest" "$lab/v1.sig" \
  "$lab/v1.pub" "$lab/test-signer-seed" 1 0 1 0
"$signer" make "$lab/v2.node" "$lab/v2.manifest" "$lab/v2.sig" \
  "$lab/v2.pub" "$lab/test-signer-seed" 2 0 2 0
cmp -s "$lab/v1.pub" "$lab/v2.pub"
"$signer" verify "$lab/v1.node" "$lab/v1.manifest" "$lab/v1.sig" "$lab/v1.pub"
pub64="$(base64 -w 0 "$lab/v1.pub")"
marker="$(openssl rand -hex 16)"
mkdir "$lab/seed"
cat > "$lab/seed/meta-data" <<META
instance-id: akula-b1b2b-$(openssl rand -hex 8)
local-hostname: akula-b1b2b
META
cat > "$lab/seed/user-data" <<CLOUD
#cloud-config
hostname: akula-b1b2b
ssh_deletekeys: true
ssh_genkeytypes: []
ssh_keys:
  ed25519_private: |
$(sed 's/^/    /' "$lab/host")
  ed25519_public: $host_pub
ssh_pwauth: false
disable_root: true
users:
  - name: akulavm
    shell: /bin/bash
    lock_passwd: true
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
      - $client_pub
write_files:
  - path: /etc/akula-b1b2b-lab-public.ed25519
    owner: root:root
    permissions: '0600'
    encoding: b64
    content: $pub64
  - path: /etc/akula-b1b2b-lab.marker
    owner: root:root
    permissions: '0600'
    content: '$marker'
  - path: /etc/ssh/sshd_config.d/90-akula-b1b2b-no-password.conf
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
(
  cd "$lab/seed"
  genisoimage -quiet -output "$lab/seed.iso" -volid cidata -rock -joliet \
    user-data meta-data
)
url=https://cloud-images.ubuntu.com/noble/current
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 \
  "$url/SHA256SUMS" -o "$lab/SHA256SUMS"
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 \
  "$url/SHA256SUMS.gpg" -o "$lab/SHA256SUMS.gpg"
gpgv --keyring /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg \
  "$lab/SHA256SUMS.gpg" "$lab/SHA256SUMS"
sha="$(awk '$2 == "*noble-server-cloudimg-amd64.img" || $2 == "noble-server-cloudimg-amd64.img" {print $1; exit}' "$lab/SHA256SUMS")"
[[ "$sha" =~ ^[0-9a-fA-F]{64}$ ]]
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 \
  "$url/noble-server-cloudimg-amd64.img" -o "$lab/guest.qcow2"
printf '%s  %s\n' "$sha" "$lab/guest.qcow2" | sha256sum --check --status
qemu-img info "$lab/guest.qcow2" >/dev/null
qemu-img resize "$lab/guest.qcow2" 8G >/dev/null
ssh_port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(("127.0.0.1",0))
    print(s.getsockname()[1])
PY
)"
accelerator=tcg
if [[ -r /dev/kvm && -w /dev/kvm ]]; then accelerator=kvm; fi
echo "B1B2B_QEMU_ACCELERATOR=$accelerator"
qemu-system-x86_64 -machine "accel=$accelerator" -m 2048 -smp 2 \
  -display none -monitor none -serial "file:$lab/console.log" \
  -drive "if=virtio,file=$lab/guest.qcow2,format=qcow2,snapshot=on" \
  -drive "if=virtio,file=$lab/seed.iso,format=raw" \
  -netdev "user,id=labnet,restrict=on,hostfwd=tcp:127.0.0.1:$ssh_port-:22" \
  -device virtio-net-pci,netdev=labnet > "$lab/qemu.log" 2>&1 &
guest_pid="$!"
printf 'akula-b1b2b %s\n' "$host_pub" > "$lab/known_hosts"
chmod 0600 "$lab/known_hosts"
ssh_lab(){
  ssh -F /dev/null -T -o BatchMode=yes -o StrictHostKeyChecking=yes \
    -o "UserKnownHostsFile=$lab/known_hosts" -o GlobalKnownHostsFile=/dev/null \
    -o HostKeyAlias=akula-b1b2b -o HostKeyAlgorithms=ssh-ed25519 \
    -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no \
    -o IdentityAgent=none -o IdentitiesOnly=yes -o ConnectTimeout=3 \
    -o ConnectionAttempts=1 -i "$lab/client" -p "$ssh_port" \
    -l akulavm 127.0.0.1 "$@"
}
ready=false
for i in $(seq 1 260); do
  if ! kill -0 "$guest_pid" 2>/dev/null; then
    echo "B1B2B_VM_QEMU_EXITED" >&2
    exit 1
  fi
  if ssh_lab true </dev/null >/dev/null 2>&1; then ready=true; break; fi
  sleep 2
done
test "$ready" = true
# Ubuntu cloud-init may return status 2 for unrelated warnings. Do not infer
# trusted provisioning from its exit status; independently prove the *exact*
# pre-boot public key and guest marker are root-controlled and match the runner.
ssh_lab "sudo cloud-init status --wait || sudo cloud-init status --long || true"
expected_key_sha="$(sha256sum "$lab/v1.pub" | cut -d ' ' -f1)"
ssh_lab "sudo test -f /etc/akula-b1b2b-lab.marker &&
  test \"\$(sudo sha256sum /etc/akula-b1b2b-lab-public.ed25519 | cut -d ' ' -f1)\" = '$expected_key_sha'"
echo "B1B2B_VM_PREBOOT_KEY_TRUST_VERIFIED"
ssh_lab "mkdir -m 0700 /home/akulavm/b1b2b"
scp -q -F /dev/null -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -o "UserKnownHostsFile=$lab/known_hosts" -o GlobalKnownHostsFile=/dev/null \
  -o HostKeyAlias=akula-b1b2b -o HostKeyAlgorithms=ssh-ed25519 \
  -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no \
  -o IdentityAgent=none -o IdentitiesOnly=yes -P "$ssh_port" -i "$lab/client" \
  "$lab/v1.node" "$lab/v2.node" "$lab/v1.manifest" "$lab/v2.manifest" \
  "$lab/v1.sig" "$lab/v2.sig" "$lab/node-token" \
  .artifacts/cargo-native/debug/examples/akula_b1b2b_release_fixture \
  scripts/ci/akula-b1b2b-guest-lab.sh scripts/ci/akula-b1b2b-node-wrapper.sh \
  akulavm@127.0.0.1:/home/akulavm/b1b2b/
ssh_lab "cd /home/akulavm/b1b2b &&
  mv akula_b1b2b_release_fixture b1a-fixture &&
  mv akula-b1b2b-node-wrapper.sh wrapper &&
  chmod 0555 b1a-fixture wrapper &&
  sudo bash akula-b1b2b-guest-lab.sh bootstrap"
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh negative"
# Hard-kill the installer after atomic activation, then recover in a new process.
set +e
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh crash"
crash_rc=$?
set -e
if (( crash_rc == 0 )); then
  echo "B1B2B_REFUSED: crash injection did not kill installer" >&2
  exit 1
fi
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh recover"
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh cleanup"
echo "B1B2B_PASS: GPG-pinned Ubuntu VM, B1a Ed25519 Node, systemd, invalid signed upgrade rollback, SIGKILL recovery"
