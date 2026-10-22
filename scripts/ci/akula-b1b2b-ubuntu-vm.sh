#!/usr/bin/env bash
# B1b2b disposable Ubuntu QEMU signed Node upgrade/rollback lab.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "$(id -u)" == 0 || "$GITHUB_ACTIONS" != true || "$RUNNER_OS" != Linux ]]; then
  echo "B1B2B_REFUSED: CI-only unprivileged runner" >&2
  exit 2
fi
vm_total_start=$SECONDS
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
active_ssh_pid=""
reboot_ssh_pid=""
cleanup(){
  result=$?
  trap - EXIT
  if (( result != 0 )); then
    echo "B1B2B_VM_FAILURE_WITNESS" >&2
    test ! -f "$lab/console.log" || tail -n 80 "$lab/console.log" >&2
    test ! -f "$lab/qemu.log" || tail -n 30 "$lab/qemu.log" >&2
  fi
  if [[ -n "$reboot_ssh_pid" ]]; then
    kill "$reboot_ssh_pid" 2>/dev/null || true
    wait "$reboot_ssh_pid" 2>/dev/null || true
  fi
  if [[ -n "$active_ssh_pid" ]]; then
    kill "$active_ssh_pid" 2>/dev/null || true
    wait "$active_ssh_pid" 2>/dev/null || true
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
image_fetch_start=$SECONDS
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
echo "B1B2B_IMAGE_FETCH_VERIFY_SECONDS=$((SECONDS - image_fetch_start))"
ssh_port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(("127.0.0.1",0))
    print(s.getsockname()[1])
PY
)"
# GitHub's disposable x86_64 Linux runner may expose /dev/kvm but leave
# it root/kvm-owned. Grant the current non-root CI group access only to this
# temporary device, never to the guest disk or release trust anchors.
# If nested virtualization is unavailable, preserve the original TCG path.
accelerator=tcg
if [[ -c /dev/kvm ]]; then
  if [[ ! -r /dev/kvm || ! -w /dev/kvm ]]; then
    if sudo chgrp "$(id -gn)" /dev/kvm && sudo chmod g+rw /dev/kvm; then
      echo "B1B2B_KVM_CI_GROUP_ACCESS_ENABLED"
    else
      echo "B1B2B_KVM_PERMISSION_FALLBACK"
    fi
  fi
  if [[ -r /dev/kvm && -w /dev/kvm ]]; then accelerator=kvm; fi
fi
echo "B1B2B_QEMU_ACCELERATOR_REQUESTED=$accelerator"
# Let QEMU attempt KVM and automatically fall back to TCG if KVM ioctl fails.
# No change to crash-recovery, disk, or guest trust invariants.
if [[ "$accelerator" == kvm ]]; then accelerator=kvm:tcg; fi
echo "B1B2B_QEMU_ACCELERATOR_POLICY=$accelerator"
# A disposable, signature-verified image COPY is the ONLY writable guest disk.
# snapshot=off is essential: forced QEMU death must not discard the entire
# guest disk and accidentally test a fresh machine instead of persisted state.
start_qemu(){
  qemu-system-x86_64 -machine "accel=$accelerator" -m 2048 -smp 2 \
    -display none -monitor none -serial "file:$lab/console.log" \
    -drive "if=virtio,file=$lab/guest.qcow2,format=qcow2,snapshot=off" \
    -drive "if=virtio,file=$lab/seed.iso,format=raw" \
    -netdev "user,id=labnet,restrict=on,hostfwd=tcp:127.0.0.1:$ssh_port-:22" \
    -device virtio-net-pci,netdev=labnet > "$lab/qemu.log" 2>&1 &
  guest_pid="$!"
}
first_boot_start=$SECONDS
start_qemu
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
# Fail closed on invalid cloud-config; a successful SSH handshake alone is
# not evidence that the entire cloud-init provisioning schema is valid.
ssh_lab "sudo cloud-init status --wait"
ssh_lab "sudo cloud-init schema --system --annotate"
echo "B1B2B_VM_CLOUD_INIT_SCHEMA_PASS"
expected_key_sha="$(sha256sum "$lab/v1.pub" | cut -d ' ' -f1)"
ssh_lab "sudo test -f /etc/akula-b1b2b-lab.marker &&
  test \"\$(sudo sha256sum /etc/akula-b1b2b-lab-public.ed25519 | cut -d ' ' -f1)\" = '$expected_key_sha'"
echo "B1B2B_VM_PREBOOT_KEY_TRUST_VERIFIED"
echo "B1B2B_FIRST_BOOT_VERIFY_SECONDS=$((SECONDS - first_boot_start))"
# Optional, non-blocking diagnostic: does not change cloud-init acceptance.
ssh_lab "sudo cloud-init analyze blame" 2>/dev/null | head -n 16 || true
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
# B1c1 runs ONLY in the explicit disposable QEMU bridge workflow.
# Default B1b2b signed Node lab remains behaviorally unchanged.
if [[ "${AKULA_B1C1_VM_BRIDGE:-}" == "SIGNED_ELF_TO_ROOT_CAS" ]]; then
  source scripts/ci/akula-b1c1-vm-hook.sh
elif [[ -n "${AKULA_B1C1_VM_BRIDGE:-}" ]]; then
  echo "B1C1_REFUSED: unknown VM extension selector" >&2
  exit 2
fi
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh negative"
# Hard-kill the installer after atomic activation, then reboot the entire
# guest OS BEFORE recovery. This is an OS reboot, not a VM power cut.
boot_before="$(ssh_lab cat /proc/sys/kernel/random/boot_id)"
[[ "$boot_before" =~ ^[0-9a-f-]{36}$ ]]
set +e
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh crash"
crash_rc=$?
set -e
if (( crash_rc == 0 )); then
  echo "B1B2B_REFUSED: crash injection did not kill installer" >&2
  exit 1
fi
echo "B1B2B_VM_INSTALLER_SIGKILL_CONFIRMED"
# ssh may exit 0 or nonzero as the kernel drops its session during reboot;
# prove the boot changed by independently reading the Linux boot_id afterward.
os_reboot_start=$SECONDS
# SSH may block after the guest is already restarting (observed 303 s in
# PR #561). Do not await the control session before checking the actual OS
# boot ID; the independently proven new boot ID remains the sole success gate.
ssh_lab "sudo systemctl reboot" </dev/null >/dev/null 2>&1 &
reboot_ssh_pid=$!
boot_after=""
reboot_verified=false
for i in $(seq 1 260); do
  if ! kill -0 "$guest_pid" 2>/dev/null; then
    echo "B1B2B_REFUSED: QEMU terminated during OS reboot" >&2
    exit 1
  fi
  boot_after="$(ssh_lab cat /proc/sys/kernel/random/boot_id 2>/dev/null || true)"
  if [[ "$boot_after" =~ ^[0-9a-f-]{36}$ && "$boot_after" != "$boot_before" ]]; then
    reboot_verified=true
    break
  fi
  sleep 2
done
if [[ "$reboot_verified" != true ]]; then
  echo "B1B2B_REFUSED: guest OS reboot was not independently proved" >&2
  exit 1
fi
# The detached host SSH client may still be awaiting a vanished session;
# terminate it only AFTER guest boot identity has been proved independently.
kill "$reboot_ssh_pid" 2>/dev/null || true
wait "$reboot_ssh_pid" 2>/dev/null || true
reboot_ssh_pid=""
echo "B1B2B_VM_OS_BOOT_ID_CHANGED"
echo "B1B2B_OS_REBOOT_SECONDS=$((SECONDS - os_reboot_start))"
# No unconfirmed candidate may auto-start during guest reboot.
if ssh_lab "sudo systemctl is-active --quiet akula-b1b2b-lab.service"; then
  echo "B1B2B_REFUSED: unconfirmed candidate auto-started before recovery" >&2
  exit 1
fi
ssh_lab "sudo test -f /opt/akula-b1b2b-lab/in-progress &&
  test \"\$(sudo readlink /opt/akula-b1b2b-lab/active)\" = releases/0002"
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh recover"
echo "B1B2B_VM_OS_REBOOT_RECOVERY_PASS"

# Separate failure mode: kill the WHOLE virtual machine while the guest
# installer process is still alive after changing the durable active pointer.
# No systemctl reboot, guest cleanup, or graceful guest shutdown is allowed.
boot_before_poweroff="$(ssh_lab cat /proc/sys/kernel/random/boot_id)"
[[ "$boot_before_poweroff" =~ ^[0-9a-f-]{36}$ ]]
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh forceoff-hold" \
  >"$lab/forceoff-witness.log" 2>&1 &
active_ssh_pid="$!"
barrier_seen=false
for i in $(seq 1 150); do
  if grep -Fq "B1B2B_VM_FORCEOFF_BARRIER_AT_V2" "$lab/forceoff-witness.log"; then
    barrier_seen=true
    break
  fi
  if ! kill -0 "$guest_pid" 2>/dev/null ||
     ! kill -0 "$active_ssh_pid" 2>/dev/null; then
    echo "B1B2B_REFUSED: QEMU or installer exited before forceoff barrier" >&2
    exit 1
  fi
  sleep 0.2
done
if [[ "$barrier_seen" != true ]]; then
  echo "B1B2B_REFUSED: no guest evidence of in-progress candidate at forceoff" >&2
  exit 1
fi
echo "B1B2B_VM_FORCEOFF_BARRIER_AT_V2"
# A SIGKILL to the host QEMU process drops the *entire* Linux guest
# immediately, without an in-guest systemctl stop or reboot.
kill -KILL "$guest_pid"
set +e
wait "$guest_pid"
qemu_exit=$?
set -e
guest_pid=""
if (( qemu_exit != 137 )); then
  echo "B1B2B_REFUSED: QEMU did not terminate from SIGKILL ($qemu_exit)" >&2
  exit 1
fi
echo "B1B2B_VM_QEMU_FORCEOFF_CONFIRMED"
kill "$active_ssh_pid" 2>/dev/null || true
wait "$active_ssh_pid" 2>/dev/null || true
active_ssh_pid=""
cp "$lab/console.log" "$lab/console.pre-forceoff.log"
hardoff_reboot_start=$SECONDS
# Restart the same disposable qcow2 from scratch. The image is not pristine:
# systemd unit, trusted key, signed artifacts and journal must survive.
start_qemu
ready_after_forceoff=false
for i in $(seq 1 260); do
  if ! kill -0 "$guest_pid" 2>/dev/null; then
    echo "B1B2B_REFUSED: QEMU died while restarting existing guest disk" >&2
    exit 1
  fi
  reboot_id="$(ssh_lab cat /proc/sys/kernel/random/boot_id 2>/dev/null || true)"
  if [[ "$reboot_id" =~ ^[0-9a-f-]{36}$ &&
        "$reboot_id" != "$boot_before_poweroff" ]]; then
    ready_after_forceoff=true
    break
  fi
  sleep 2
done
if [[ "$ready_after_forceoff" != true ]]; then
  echo "B1B2B_REFUSED: no new kernel boot on same qcow2 after hardoff" >&2
  exit 1
fi
echo "B1B2B_VM_HARDOFF_BOOT_ID_CHANGED"
echo "B1B2B_HARDOFF_REBOOT_SECONDS=$((SECONDS - hardoff_reboot_start))"
# ssh_lab always performs strict host-key pinning. Check the *release*
# trust root was also preserved and did not silently fall back to a new key.
ssh_lab "sudo cloud-init status --wait && sudo cloud-init schema --system --annotate"
ssh_lab "sudo test -f /etc/akula-b1b2b-lab.marker &&
  test \"\$(sudo sha256sum /etc/akula-b1b2b-lab-public.ed25519 | cut -d ' ' -f1)\" = '$expected_key_sha'"
if ssh_lab "sudo systemctl is-active --quiet akula-b1b2b-lab.service"; then
  echo "B1B2B_REFUSED: unconfirmed signed candidate started during hardoff restart" >&2
  exit 1
fi
ssh_lab "sudo test -f /opt/akula-b1b2b-lab/in-progress &&
  test \"\$(sudo readlink /opt/akula-b1b2b-lab/active)\" = releases/0002"
echo "B1B2B_VM_HARDOFF_PERSISTENCE_PASS"
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh recover"
echo "B1B2B_VM_HARDOFF_SIGNED_RECOVERY_PASS"
ssh_lab "sudo bash /home/akulavm/b1b2b/akula-b1b2b-guest-lab.sh cleanup"
echo "B1B2B_PASS: schema-valid Ubuntu VM, signed Node, SIGKILL installer, OS reboot, QEMU SIGKILL and signed recovery"
echo "B1B2B_TOTAL_SCRIPT_SECONDS=$((SECONDS - vm_total_start))"
