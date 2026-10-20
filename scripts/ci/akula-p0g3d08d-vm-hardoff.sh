#!/usr/bin/env bash
set -euo pipefail

# P0g-3d-08d-2a: REAL QEMU VM force-off at the native test-only after-V8 seam.
# No user vault, S3 credentials, production publisher or physical power cut.
cd "$(dirname "$0")/../.."
if [[ "$(id -u)" == 0 ]]; then
  echo "Refuse to execute a VM hard-off lab as root on the host" >&2
  exit 2
fi
for program in curl gpgv sha256sum openssl ssh scp ssh-keygen sudo python3 awk grep mkfs.ext4 qemu-system-x86_64 qemu-img genisoimage; do
  if ! command -v "$program" >/dev/null; then
    echo "Missing VM lab prerequisite: $program" >&2
    exit 2
  fi
done
test -r /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg

lab="$(mktemp -d /tmp/akula-p0g3d08d-vm.XXXXXXXX)"
chmod 700 "$lab"
guest_pid=""
remote_pid=""
cleanup() {
  status=$?
  trap - EXIT
  if [[ -n "$guest_pid" ]]; then
    kill -KILL "$guest_pid" 2>/dev/null || true
    wait "$guest_pid" 2>/dev/null || true
  fi
  if [[ -n "$remote_pid" ]]; then
    kill "$remote_pid" 2>/dev/null || true
    wait "$remote_pid" 2>/dev/null || true
  fi
  if [[ "$status" -ne 0 ]]; then
    echo "AKULA_P0G3D08D_VM_LAB_FAILED: $status" >&2
    for file in "$lab"/qemu-*.log "$lab"/console-*.log "$lab"/writer.log "$lab"/recovery.log; do
      if [[ -f "$file" ]]; then
        echo "--- $file (last 80 lines) ---" >&2
        tail -n 80 "$file" >&2
      fi
    done
  fi
  rm -rf -- "$lab"
  exit "$status"
}
trap cleanup EXIT

ssh-keygen -q -t ed25519 -N '' -f "$lab/host"
ssh-keygen -q -t ed25519 -N '' -f "$lab/client"
chmod 600 "$lab/host" "$lab/client"
host_pub="$(cut -d ' ' -f 1,2 "$lab/host.pub")"
client_pub="$(cut -d ' ' -f 1,2 "$lab/client.pub")"
mkdir "$lab/seed"
cat > "$lab/seed/meta-data" <<META
instance-id: akula-sync-hardoff-$(openssl rand -hex 10)
local-hostname: akula-sync-hardoff
META
cat > "$lab/seed/user-data" <<CLOUD
#cloud-config
hostname: akula-sync-hardoff
ssh_deletekeys: true
ssh_genkeytypes: []
ssh_keys:
  ed25519_private: |
$(sed 's/^/    /' "$lab/host")
  ed25519_public: $host_pub
disable_root: true
ssh_pwauth: false
users:
  - name: akulavm
    shell: /bin/bash
    lock_passwd: true
    groups: [sudo]
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - $client_pub
write_files:
  - path: /etc/ssh/sshd_config.d/90-akula-no-password.conf
    owner: root:root
    permissions: '0644'
    content: |
      PasswordAuthentication no
      KbdInteractiveAuthentication no
      PermitRootLogin no
      AllowTcpForwarding no
runcmd:
  - [systemctl, restart, ssh]
CLOUD
(
  cd "$lab/seed"
  genisoimage -quiet -output "$lab/seed.iso" -volid cidata -rock -joliet user-data meta-data
)

# Signed Ubuntu cloud-image manifest authenticates the guest image only.
# It does NOT authenticate Akula or prove a storage controller's durability.
base_url="https://cloud-images.ubuntu.com/noble/current"
image="noble-server-cloudimg-amd64.img"
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 "$base_url/SHA256SUMS" -o "$lab/SHA256SUMS"
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 "$base_url/SHA256SUMS.gpg" -o "$lab/SHA256SUMS.gpg"
gpgv --keyring /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg "$lab/SHA256SUMS.gpg" "$lab/SHA256SUMS"
checksum="$(awk '$2 == "*noble-server-cloudimg-amd64.img" || $2 == "noble-server-cloudimg-amd64.img" {print $1; exit}' "$lab/SHA256SUMS")"
if [[ ! "$checksum" =~ ^[[:xdigit:]]{64}$ ]]; then
  echo "Signed Ubuntu checksum missing" >&2
  exit 1
fi
curl -fsSL --retry 3 --connect-timeout 20 --max-time 900 "$base_url/$image" -o "$lab/guest.qcow2"
printf '%s  %s\n' "$checksum" "$lab/guest.qcow2" | sha256sum --check --status
qemu-img resize "$lab/guest.qcow2" 8G >/dev/null

# This is the ONLY persistence device under test. Host never mounts it.
qemu-img create -q -f raw "$lab/volume.raw" 4G
mkfs.ext4 -F -q -E lazy_itable_init=0,lazy_journal_init=0 -L AKULA_SYNC_VM "$lab/volume.raw"
echo "AKULA_VM_IMAGE_SIGNED_SHA256=$checksum"
echo "AKULA_VM_DISK_FORMAT=ext4 QEMU_CACHE_MODE=none HOST_STORAGE=ephemeral"

# Build EXACT Rust libtest binary from the checked-out PR, not a fake writer.
(
  cd aquilum-app
  cargo test --locked -p aquilum-core --features sync-s3 --lib --no-run --message-format=json > "$lab/build.json"
)
test_bin="$(python3 - "$lab/build.json" <<'PY'
import json, sys
found = []
for line in open(sys.argv[1], encoding="utf-8"):
    try:
        x = json.loads(line)
    except ValueError:
        continue
    if x.get("reason") != "compiler-artifact":
        continue
    t = x.get("target") or {}
    if t.get("name") == "aquilum_core" and x.get("executable"):
        found.append(x["executable"])
if len(found) != 1:
    raise SystemExit(f"Expected one compiled Core libtest, found {len(found)}")
print(found[0])
PY
)"
test -x "$test_bin"
"$test_bin" --list > "$lab/test-list.txt"
grep -Fq "p0g3d08c_writer_abort_entrypoint: test" "$lab/test-list.txt"
grep -Fq "p0g3d08_os_process_probe_entrypoint: test" "$lab/test-list.txt"

port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(("127.0.0.1", 0))
    print(s.getsockname()[1])
PY
)"
printf 'akula-sync-hardoff %s\n' "$host_pub" > "$lab/known_hosts"
chmod 600 "$lab/known_hosts"
ssh_guest() {
  ssh -F /dev/null -T -o BatchMode=yes -o StrictHostKeyChecking=yes \
      -o "UserKnownHostsFile=$lab/known_hosts" -o GlobalKnownHostsFile=/dev/null \
      -o HostKeyAlias=akula-sync-hardoff -o HostKeyAlgorithms=ssh-ed25519 \
      -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no \
      -o IdentityAgent=none -o IdentitiesOnly=yes -o ConnectTimeout=3 \
      -i "$lab/client" -p "$port" -l akulavm -- 127.0.0.1 "$@"
}
scp_guest() {
  scp -q -F /dev/null -o BatchMode=yes -o StrictHostKeyChecking=yes \
      -o "UserKnownHostsFile=$lab/known_hosts" -o GlobalKnownHostsFile=/dev/null \
      -o HostKeyAlias=akula-sync-hardoff -o HostKeyAlgorithms=ssh-ed25519 \
      -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no \
      -o IdentitiesOnly=yes -o ConnectTimeout=5 -i "$lab/client" -P "$port" \
      "$1" "akulavm@127.0.0.1:$2"
}
accelerator=tcg
if [[ -r /dev/kvm && -w /dev/kvm ]]; then accelerator=kvm; fi
echo "AKULA_VM_ACCELERATOR=$accelerator"

boot_guest() {
  boot="$1"
  qemu-system-x86_64 \
    -machine "accel=$accelerator" -m 3072 -smp 2 \
    -display none -monitor none -serial "file:$lab/console-$boot.log" \
    -drive "if=virtio,file=$lab/guest.qcow2,format=qcow2,snapshot=on" \
    -drive "if=virtio,file=$lab/volume.raw,format=raw,cache=none" \
    -drive "if=virtio,file=$lab/seed.iso,format=raw" \
    -netdev "user,id=labnet,restrict=on,hostfwd=tcp:127.0.0.1:$port-:22" \
    -device virtio-net-pci,netdev=labnet \
    > "$lab/qemu-$boot.log" 2>&1 &
  guest_pid="$!"
  ready=false
  for attempt in $(seq 1 190); do
    if ! kill -0 "$guest_pid" 2>/dev/null; then
      echo "QEMU exited before guest SSH was ready at boot $boot" >&2
      exit 1
    fi
    if ssh_guest true </dev/null >/dev/null 2>&1; then
      ready=true
      break
    fi
    sleep 2
  done
  if [[ "$ready" != true ]]; then
    echo "Guest failed SSH readiness at boot $boot" >&2
    exit 1
  fi
  # The second virtio-blk drive is the ONLY guest data disk. Validate mount
  # identity rather than ever assuming an arbitrary preexisting host path.
  ssh_guest 'sudo mkdir -p /mnt/akula-hardoff &&
    sudo mount -t ext4 -o nosuid,nodev /dev/vdb /mnt/akula-hardoff &&
    test "$(findmnt -n -o SOURCE /mnt/akula-hardoff)" = "/dev/vdb" &&
    sudo chown akulavm:akulavm /mnt/akula-hardoff'
}

boot_guest first
scp_guest "$test_bin" "/mnt/akula-hardoff/akula-core-test"
ssh_guest 'chmod 700 /mnt/akula-hardoff/akula-core-test && sync'
# Real native File Gate + HistoryService + V7/V8 test writer will PAUSE, not
# exit or abort. The host is the only actor that kills the entire VM.
ssh_guest 'env AKULA_P0G3D08C_STAGE=after-v8 \
    AKULA_P0G3D08C_WORKDIR=/mnt/akula-hardoff \
    AKULA_P0G3D08D_VM_PAUSE=1 \
    /mnt/akula-hardoff/akula-core-test --exact \
    sync::s3_reserved_replace::tests::p0g3d08c_writer_abort_entrypoint \
    --nocapture --test-threads=1' > "$lab/writer.log" 2>&1 &
remote_pid="$!"
witness=false
for attempt in $(seq 1 240); do
  if grep -Fq 'AKULA_P0G3D08D_VM_READY=after-v8' "$lab/writer.log"; then
    witness=true
    break
  fi
  if ! kill -0 "$guest_pid" 2>/dev/null || ! kill -0 "$remote_pid" 2>/dev/null; then
    echo "VM or writer exited before out-of-band seam signal" >&2
    exit 1
  fi
  sleep 1
done
if [[ "$witness" != true ]]; then
  echo "Out-of-band after-V8 witness never arrived" >&2
  exit 1
fi
python3 - "$lab/writer.log" "$lab/identifiers" <<'PY'
import json, pathlib, re, sys, uuid
text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
values = re.findall(r"AKULA_P0G3D08D_VM_FIXTURE=(\{[^\r\n]+\})", text)
if len(values) != 1:
    raise SystemExit(f"Expected one out-of-band fixture witness, found {len(values)}")
w = json.loads(values[0])
root = str(w["root"])
appdata = str(w["appdata"])
for value in (root, appdata):
    if not re.fullmatch(r"[a-zA-Z0-9/._-]+", value):
        raise SystemExit("Unsafe guest fixture path")
    if ".." in pathlib.PurePosixPath(value).parts:
        raise SystemExit("Noncanonical guest path")
if not re.fullmatch(r"/mnt/akula-hardoff/writer-crash-fixture-[a-zA-Z0-9_-]+/vault", root):
    raise SystemExit("Fixture root outside disposable VM volume")
if appdata != root.rsplit("/", 1)[0] + "/appdata":
    raise SystemExit("Wrong fixture appdata")
operation = str(uuid.UUID(str(w["operation"])))
pathlib.Path(sys.argv[2]).write_text("\n".join((root, appdata, operation)) + "\n", encoding="utf-8")
PY
mapfile -t ids < "$lab/identifiers"
if [[ "$(wc -l < "$lab/identifiers")" -ne 3 ]]; then exit 1; fi

# QEMU SIGKILL removes the *guest kernel's* volatile buffers and process
# memory. It is NOT a physical AC power cut of the host SSD/controller.
echo "AKULA_P0G3D08D_OUT_OF_BAND_SEAM=after-v8"
kill -KILL "$guest_pid"
wait "$guest_pid" 2>/dev/null || true
guest_pid=""
wait "$remote_pid" 2>/dev/null || true
remote_pid=""

# Boot a NEW Linux kernel from the original ephemeral cloud image but mount
# the SAME persistent raw ext4 disk; replay OS journal recovery normally.
boot_guest second
ssh_guest "env AKULA_P0G3D08_CHILD_MODE=vm-classify \
    AKULA_P0G3D08_ROOT='${ids[0]}' \
    AKULA_P0G3D08_APPDATA='${ids[1]}' \
    AKULA_P0G3D08_OPERATION='${ids[2]}' \
    /mnt/akula-hardoff/akula-core-test --exact \
    sync::s3_reserved_replace::tests::p0g3d08_os_process_probe_entrypoint \
    --nocapture --test-threads=1" > "$lab/recovery.log" 2>&1
if ! grep -Eq 'AKULA_P0G3D08D_VM_COLD_CLASS=(HOLD|DENY)' "$lab/recovery.log"; then
  echo "Cold process produced neither an explicit HOLD nor explicit DENY" >&2
  exit 1
fi
if ! grep -Fq 'p0g3d08_os_process_probe_entrypoint ... ok' "$lab/recovery.log"; then
  echo "Cold test filter was not executed successfully" >&2
  exit 1
fi
grep -E 'AKULA_P0G3D08D_VM_COLD_CLASS=' "$lab/recovery.log"
echo "AKULA_P0G3D08D_VM_REAL_FORCE_OFF_COLD_REOPEN_PASS"
echo "Only a guest QEMU hard-off was tested; physical power-loss durability remains unproven."
