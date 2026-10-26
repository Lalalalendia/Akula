#!/usr/bin/env bash
# B1b4b1 disposable Ubuntu systemd witness: READ CHECKPOINT only.
# Never a production service or executable installer.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "$GITHUB_ACTIONS" != true || "$RUNNER_OS" != Linux || "$(id -u)" == 0 ]]; then
  echo "B1B4B1_REFUSED: only nonroot disposable Linux CI" >&2
  exit 2
fi
for tool in sudo systemd-run systemctl journalctl python3 openssl; do
  command -v "$tool" >/dev/null
done
root_dir=/run/akula-b1b4b1-lab
opt_dir=/opt/akula-b1b4b1-lab
trust_dir="$root_dir/trust"
journal_dir="$root_dir/journal"
sock="$root_dir/witness.sock"
if [[ -e "$root_dir" || -L "$root_dir" || -e "$opt_dir" || -L "$opt_dir" ]]; then
  echo "B1B4B1_REFUSED: unexpected pre-existing OS path" >&2
  exit 2
fi
tmp="$(mktemp -d /tmp/akula-b1b4b1.XXXXXXXX)"
chmod 0700 "$tmp"
owned=false
cleanup() {
  result=$?
  trap - EXIT
  if [[ "$owned" == true ]]; then
    for unit in akula-b1b4b1-lab akula-b1b4b1-lab-missing \
        akula-b1b4b1-lab-wrongkey akula-b1b4b1-lab-restored; do
      sudo systemctl stop "$unit" >/dev/null 2>&1 || true
      sudo systemctl reset-failed "$unit" >/dev/null 2>&1 || true
    done
    sudo rm -rf -- "$root_dir" "$opt_dir"
  fi
  rm -rf -- "$tmp"
  exit "$result"
}
trap cleanup EXIT
uid="$(id -u)"
gid="$(id -g)"
test "$uid" -gt 0
openssl rand -out "$tmp/witness.seed" 32
fixture=.artifacts/cargo-native/debug/examples/akula_b1b4b1_witness_fixture
binary=.artifacts/cargo-native/debug/akula-witness-b1b4b1-lab
test -x "$fixture" && test -x "$binary"
"$fixture" "$tmp/witness.seed" "$tmp/witness.pub"
printf 'AKULA-B1B4B-CI-ENROLLED\n' > "$tmp/enrollment"
python3 - "$uid" "$tmp/allowed-peer-uid" <<'PY'
import struct,sys
with open(sys.argv[2], "wb") as out:
    out.write(struct.pack(">I", int(sys.argv[1])))
PY
sudo install -d -o root -g root -m 0711 "$root_dir"
owned=true
sudo install -d -o root -g root -m 0700 "$trust_dir"
sudo install -d -o root -g root -m 0700 "$journal_dir"
sudo install -d -o root -g root -m 0755 "$opt_dir"
for name in witness.seed witness.pub enrollment; do
  sudo install -o root -g root -m 0600 "$tmp/$name" "$trust_dir/$name"
done
sudo install -o root -g root -m 0600 "$tmp/allowed-peer-uid" "$root_dir/allowed-peer-uid"
sudo install -o root -g root -m 0555 "$binary" "$opt_dir/witness"
[[ "$(sudo stat -c '%u:%a' "$root_dir")" == "0:711" ]]
[[ "$(sudo stat -c '%u:%a' "$trust_dir")" == "0:700" ]]
[[ "$(sudo stat -c '%u:%a' "$journal_dir")" == "0:700" ]]
if (printf x >> "$trust_dir/witness.seed") 2>/dev/null; then
  echo "B1B4B1_REFUSED: client can modify signer" >&2
  exit 1
fi
echo "B1B4B1_OS_OWNED_NONROOT_WRITE_REFUSAL_PASS"

launch() {
  unit="$1"
  sudo systemd-run --unit="$unit" --service-type=simple \
    --property=User=root \
    --property=NoNewPrivileges=yes \
    --property=ProtectSystem=strict \
    --property=ReadWritePaths="$root_dir" \
    --property=ProtectHome=yes \
    --property=PrivateTmp=yes \
    --property=PrivateDevices=yes \
    --property=RestrictAddressFamilies=AF_UNIX \
    --property=IPAddressDeny=any \
    --property=MemoryMax=128M \
    --property=TimeoutStopSec=5s \
    "$opt_dir/witness"
}
wait_bound() {
  unit="$1"
  found=false
  for _ in $(seq 1 100); do
    if [[ -S "$sock" ]]; then found=true; break; fi
    if ! sudo systemctl is-active --quiet "$unit"; then
      echo "B1B4B1_REFUSED: witness failed before socket bind $unit" >&2
      sudo journalctl -u "$unit" -o cat -n 50 --no-pager >&2 || true
      return 1
    fi
    sleep 0.1
  done
  [[ "$found" == true ]]
  mainpid="$(sudo systemctl show "$unit" -p MainPID --value)"
  [[ "$mainpid" =~ ^[1-9][0-9]*$ ]]
  [[ "$(sudo awk '/^Uid:/ {print $3}' "/proc/$mainpid/status")" == 0 ]]
  [[ "$(sudo systemctl show "$unit" -p ProtectSystem --value)" == strict ]]
}
send() {
  expected="$1"
  kind="$2"
  caller="$3"
  if [[ "$caller" == root ]]; then
    sudo python3 "$tmp/send.py" "$sock" "$kind" "$expected"
  else
    python3 "$tmp/send.py" "$sock" "$kind" "$expected"
  fi
}
cat > "$tmp/send.py" <<'PY'
import socket,sys
request=b"AKULA-B1B4B1-READ-HEAD-V1\x00"
prefix=b"AKULA-B1B4B1-HEAD-V1\x00"
kind,expected=sys.argv[2:]
payload={"valid":request, "extra":request+b"x",
         "invalid":b"X"+request[1:], "short":request[:-1]}[kind]
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
s.settimeout(4)
s.connect(sys.argv[1])
try:
    s.sendall(payload)
except BrokenPipeError:
    pass
try:
    s.shutdown(socket.SHUT_WR)
except OSError:
    pass
chunks=[]
reset=False
while True:
    try:
        part=s.recv(4096)
    except ConnectionResetError:
        # A reject before reading the frame can close a Unix stream with
        # unread payload and produce ECONNRESET, not the denial byte.
        # Treat reset as denial ONLY for negative cases and only with
        # no prior response bytes. Successful read must return a full frame.
        reset=True
        break
    if not part: break
    chunks.append(part)
reply=b"".join(chunks)
if expected == "accept":
    assert not reset, "accepted IPC request was reset before full read"
    assert reply.startswith(prefix), f"no exact witness response: {reply!r}"
    assert len(reply) == len(prefix)+132, f"unexpected head size: {len(reply)}"
    epoch=int.from_bytes(reply[len(prefix):len(prefix)+8], "big")
    assert epoch == 1, f"test ledger must contain one durable signed CAS record: {epoch}"
    raw=reply[len(prefix):]
    assert raw[8:40] != bytes(32), "accepted CAS record tip must be nonzero"
    assert int.from_bytes(raw[40:48], "big") == 1, "floor generation mismatch"
    assert raw[48:80] != bytes(32), "chained floor tip must be nonzero"
    assert int.from_bytes(raw[80:88], "big") == 17, "release sequence mismatch"
    assert raw[100:132] != bytes(32), "signed manifest fingerprint absent"
else:
    assert reply == b"\x00" or (reset and reply == b""), (
        f"unexpected refusal response/reset={reset}: {reply!r}"
    )
PY
# Seed one real, owner-signed, fsynced B1b1c CAS record before service
# launch. The separately running UID0 witness must read that exact revision.
seeded=0
for testbinary in .artifacts/cargo-native/debug/deps/server_bootstrap_b1b4b-*; do
  if [[ -f "$testbinary" && -x "$testbinary" && "$testbinary" != *.d ]]; then
    sudo env GITHUB_ACTIONS=true \
      AKULA_B1B4B1_CI_SEED=APPROVED_DISPOSABLE_ROOT_CAS_ONLY \
      "$PWD/$testbinary" ci_optional_real_witness_ipc_checkpoint_fixture --exact --nocapture
    seeded=$((seeded + 1))
  fi
done
test "$seeded" -eq 1
[[ "$(sudo find "$journal_dir" -mindepth 1 -maxdepth 1 -name '*.record' | wc -l)" == 1 ]]
record="$journal_dir/witness-0000000000000001.record"
[[ "$(sudo stat -c '%u:%a' "$record")" == "0:600" ]]
sudo sha256sum "$record" > "$tmp/before-record.sha256"
echo "B1B4B1_REAL_ROOT_SIGNED_NONZERO_CAS_SEEDED"
unit=akula-b1b4b1-lab
launch "$unit"
wait_bound "$unit"
sudo chgrp "$gid" "$sock"
sudo chmod 0660 "$sock"
[[ "$(sudo stat -c '%u:%a' "$sock")" == "0:660" ]]
send accept valid runner
send deny valid root
# Deliberately open transport mode on this disposable CI-only socket;
# a different nonroot UID still MUST fail kernel SO_PEERCRED admission.
sudo chmod 0666 "$sock"
sudo -u nobody python3 - "$sock" <<'PY'
import socket,sys
request=b"AKULA-B1B4B1-READ-HEAD-V1\x00"
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
s.settimeout(4)
s.connect(sys.argv[1])
try:
    s.sendall(request)
except BrokenPipeError:
    pass
try:
    s.shutdown(socket.SHUT_WR)
except OSError:
    pass
try:
    refused=s.recv(2)
except ConnectionResetError:
    refused=b""
assert refused in (b"\x00", b""), (
    f"wrong nonroot peer UID admitted: {refused!r}"
)
PY
sudo chmod 0660 "$sock"
echo "B1B4B1_WRONG_NONROOT_UID_DENIED_BY_KERNEL"
send deny invalid runner
send deny extra runner
send deny short runner
[[ "$(sudo find "$journal_dir" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 1 ]]
sudo sha256sum "$record" | cmp -s "$tmp/before-record.sha256" -
echo "B1B4B1_READONLY_DOES_NOT_WRITE_CAS_PASS"
sudo journalctl -u "$unit" -o cat --no-pager > "$tmp/root.log"
grep -Fq 'B1B4B1_REAL_ROOT_WITNESS_BOUND' "$tmp/root.log"
grep -Fq 'B1B4B1_READ_ONLY' "$tmp/root.log"
grep -Fq 'B1B4B1_REFUSED' "$tmp/root.log"
# Exactly one positive and five negative requests were issued to the
# live service; require their explicit journald decisions. ECONNRESET
# alone is never sufficient evidence that a refusal handler ran.
[[ "$(grep -Fc 'B1B4B1_READ_ONLY' "$tmp/root.log")" == 1 ]]
[[ "$(grep -Fc 'B1B4B1_REFUSED' "$tmp/root.log")" == 5 ]]
sudo systemctl is-active --quiet "$unit"
echo "B1B4B1_REAL_ROOT_SYSTEMD_READ_ONLY_IPC_PASS"

stop() {
  sudo systemctl stop "$unit" >/dev/null 2>&1 || true
  sudo rm -f -- "$sock"
}
expect_failure() {
  case_name="$1"
  unit="akula-b1b4b1-lab-$case_name"
  launch "$unit"
  confirmed=false
  for _ in $(seq 1 100); do
    if [[ -S "$sock" ]]; then
      echo "B1B4B1_REFUSED: unsafe witness bound a socket ($case_name)" >&2
      exit 1
    fi
    status="$(sudo systemctl show "$unit" -p ExecMainStatus --value 2>/dev/null || true)"
    result="$(sudo systemctl show "$unit" -p Result --value 2>/dev/null || true)"
    sudo journalctl -u "$unit" -o cat --no-pager > "$tmp/$case_name.log" 2>/dev/null || true
    if [[ "$result" == exit-code && "$status" == 2 ]] &&
       grep -Fq 'B1B4B1_REFUSED: OS-owned witness refused:' "$tmp/$case_name.log"; then
      confirmed=true
      break
    fi
    sleep 0.1
  done
  [[ "$confirmed" == true ]] || {
    sudo journalctl -u "$unit" -o cat -n 50 --no-pager >&2 || true
    return 1
  }
  echo "B1B4B1_REAL_SYSTEMD_REFUSAL_$case_name"
}

unit=akula-b1b4b1-lab
stop
sudo mv "$trust_dir/enrollment" "$trust_dir/enrollment.saved"
expect_failure missing
sudo mv "$trust_dir/enrollment.saved" "$trust_dir/enrollment"

unit=akula-b1b4b1-lab-missing
stop
openssl rand -out "$tmp/wrong-witness.pub" 32
sudo install -o root -g root -m 0600 "$tmp/wrong-witness.pub" "$trust_dir/witness.pub"
expect_failure wrongkey
sudo install -o root -g root -m 0600 "$tmp/witness.pub" "$trust_dir/witness.pub"

unit=akula-b1b4b1-lab-wrongkey
stop
unit=akula-b1b4b1-lab-restored
launch "$unit"
wait_bound "$unit"
sudo chgrp "$gid" "$sock"
sudo chmod 0660 "$sock"
send accept valid runner
echo "B1B4B1_RESTORED_SYSTEMD_READ_ONLY_PASS"
echo "B1B4B1_PASS: OS-owned witness read-only IPC, actual root and exact UID checks"
